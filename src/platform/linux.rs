use anyhow::Result;
use std::sync::Mutex;
use std::thread::JoinHandle;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::config::KeyCombo;
use ksni::TrayMethods;

static TRAY_HANDLE: Mutex<Option<ksni::Handle<WflowTray>>> = Mutex::new(None);

#[derive(Debug)]
pub struct WflowTray {
    pub is_recording: bool,
    pub tx: mpsc::UnboundedSender<HotkeyEvent>,
}

impl ksni::Tray for WflowTray {
    fn id(&self) -> String {
        "wflow".into()
    }

    fn title(&self) -> String {
        "Wflow Voice Dictation".into()
    }

    fn icon_name(&self) -> String {
        if self.is_recording {
            "media-record".into()
        } else {
            "audio-input-microphone".into()
        }
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        let bytes: &[u8] = if self.is_recording {
            include_bytes!("../../assets/tray_rec_32.png").as_slice()
        } else {
            include_bytes!("../../assets/tray_32.png").as_slice()
        };
        if let Ok(img) = image::load_from_memory(bytes) {
            let width = img.width() as i32;
            let height = img.height() as i32;
            let mut data = img.into_rgba8().into_vec();
            for pixel in data.chunks_exact_mut(4) {
                pixel.rotate_right(1); // RGBA -> ARGB network byte order
            }
            vec![ksni::Icon { width, height, data }]
        } else {
            Vec::new()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.tx.send(HotkeyEvent::ToggleWindow);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::*;
        let tx1 = self.tx.clone();
        let tx2 = self.tx.clone();
        let is_rec = self.is_recording;

        vec![
            StandardItem {
                label: "เปิด / ซ่อน Wflow Studio (Show / Hide)".into(),
                icon_name: "view-restore".into(),
                activate: Box::new(move |_| {
                    let _ = tx1.send(HotkeyEvent::ToggleWindow);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: if is_rec {
                    "⏹️ หยุดบันทึกเสียง (Stop Dictation)".into()
                } else {
                    "🎙️ เริ่มบันทึกเสียง (Start Dictation)".into()
                },
                icon_name: if is_rec {
                    "media-playback-stop".into()
                } else {
                    "media-record".into()
                },
                activate: Box::new(move |_| {
                    if is_rec {
                        let _ = tx2.send(HotkeyEvent::Released);
                    } else {
                        let _ = tx2.send(HotkeyEvent::Pressed);
                    }
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "🔴 ปิดโปรแกรม (Quit Wflow)".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(|_| {
                    std::process::exit(0);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

#[derive(Clone, Debug, Default)]
pub struct ActiveAppContext {
    pub process_name: String,
    pub window_title: String,
    pub before_text: Option<String>,
    pub after_text: Option<String>,
    pub selected_text: Option<String>,
    pub full_text: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum HotkeyEvent {
    Pressed,
    Released,
    ToggleWindow,
    QuitApp,
    KeyRecorded(u8, KeyCombo),
    AutostartToggled(bool),
}

pub fn vk_to_string(vk: u16) -> String {
    match vk {
        0x70 => "F1".to_string(),
        0x71 => "F2".to_string(),
        0x72 => "F3".to_string(),
        0x73 => "F4".to_string(),
        0x74 => "F5".to_string(),
        0x75 => "F6".to_string(),
        0x76 => "F7".to_string(),
        0x77 => "F8".to_string(),
        0x78 => "F9".to_string(),
        0x79 => "F10".to_string(),
        0x7A => "F11".to_string(),
        0x7B => "F12".to_string(),
        0xA2 => "Left Control".to_string(),
        0xA3 => "Right Control".to_string(),
        0xA4 => "Left Alt".to_string(),
        0xA5 => "Right Alt".to_string(),
        0x14 => "Caps Lock".to_string(),
        0x20 => "Space".to_string(),
        0x30..=0x39 => format!("{}", (vk as u8) as char),
        0x41..=0x5A => format!("{}", (vk as u8) as char),
        other => format!("Key(0x{:02X})", other),
    }
}

static RECORDING_MODE: Mutex<Option<u8>> = Mutex::new(None);
static HOTKEY_VK: Mutex<u16> = Mutex::new(0x77); // F8
static TOGGLE_VK: Mutex<u16> = Mutex::new(0x78); // F9

pub struct PlatformManager;

impl PlatformManager {
    pub fn get_active_app_context() -> ActiveAppContext {
        // Try reading window title on Linux via xdotool if available
        let title_output = std::process::Command::new("xdotool")
            .args(&["getactivewindow", "getwindowname"])
            .output();

        let window_title = match title_output {
            Ok(out) if out.status.success() => {
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            }
            _ => String::new(),
        };

        let pid_output = std::process::Command::new("xdotool")
            .args(&["getactivewindow", "getwindowpid"])
            .output();

        let process_name = match pid_output {
            Ok(out) if out.status.success() => {
                let pid_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if let Ok(comm) = std::fs::read_to_string(format!("/proc/{}/comm", pid_str)) {
                    comm.trim().to_string()
                } else {
                    String::new()
                }
            }
            _ => String::new(),
        };

        ActiveAppContext {
            process_name,
            window_title,
            before_text: None,
            after_text: None,
            selected_text: None,
            full_text: None,
        }
    }

    pub fn paste_text(text: &str) -> Result<()> {
        if let Ok(mut clipboard) = arboard::Clipboard::new() {
            let _ = clipboard.set_text(text.to_string());
        }

        // Simulate Ctrl+V on Linux via xdotool if installed
        let _ = std::process::Command::new("xdotool")
            .args(&["key", "ctrl+v"])
            .status();

        Ok(())
    }

    pub fn start_hotkey_listener(
        tx: mpsc::UnboundedSender<HotkeyEvent>,
        initial_hotkey: KeyCombo,
        initial_toggle: KeyCombo,
    ) -> Result<JoinHandle<()>> {
        if let Ok(mut lock) = HOTKEY_VK.lock() {
            *lock = initial_hotkey.key;
        }
        if let Ok(mut lock) = TOGGLE_VK.lock() {
            *lock = initial_toggle.key;
        }

        // Spawn Linux System Tray via ksni in background thread
        let tray_tx = tx.clone();
        let tray = WflowTray {
            is_recording: false,
            tx: tray_tx,
        };

        std::thread::spawn(move || {
            match async_io::block_on(tray.spawn()) {
                Ok(handle) => {
                    info!("Linux StatusNotifierItem System Tray started successfully!");
                    if let Ok(mut lock) = TRAY_HANDLE.lock() {
                        *lock = Some(handle);
                    }
                }
                Err(err) => {
                    warn!("Could not start Linux StatusNotifierItem System Tray: {:?}", err);
                }
            }
        });

        info!("Starting Linux background hotkey listener thread...");
        let handle = std::thread::spawn(move || {
            while !tx.is_closed() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });

        Ok(handle)
    }

    #[allow(dead_code)]
    pub fn get_hotkey_vk() -> u16 {
        *HOTKEY_VK.lock().unwrap()
    }

    #[allow(dead_code)]
    pub fn get_toggle_window_vk() -> u16 {
        *TOGGLE_VK.lock().unwrap()
    }

    #[allow(dead_code)]
    pub fn get_hotkey_combo() -> KeyCombo {
        KeyCombo::single(*HOTKEY_VK.lock().unwrap())
    }

    #[allow(dead_code)]
    pub fn get_toggle_window_combo() -> KeyCombo {
        KeyCombo::single(*TOGGLE_VK.lock().unwrap())
    }

    pub fn update_hotkeys(hotkey: KeyCombo, toggle: KeyCombo) {
        if let Ok(mut lock) = HOTKEY_VK.lock() {
            *lock = hotkey.key;
        }
        if let Ok(mut lock) = TOGGLE_VK.lock() {
            *lock = toggle.key;
        }
    }

    pub fn start_recording_hotkey(mode: u8) {
        if let Ok(mut lock) = RECORDING_MODE.lock() {
            *lock = Some(mode);
        }
    }

    pub fn stop_recording_hotkey() {
        if let Ok(mut lock) = RECORDING_MODE.lock() {
            *lock = None;
        }
    }

    pub fn show_floating_hud(_badge: &str) {}
    pub fn set_hud_status(_status: crate::ui::DictationStatus) {}
    pub fn update_hud_amplitude(_amp: f32) {}
    pub fn hide_floating_hud() {}

    pub fn set_tray_recording_state(is_recording: bool) {
        if let Ok(lock) = TRAY_HANDLE.lock() {
            if let Some(handle) = lock.as_ref() {
                let h = handle.clone();
                std::thread::spawn(move || {
                    async_io::block_on(async {
                        h.update(|tray: &mut WflowTray| {
                            tray.is_recording = is_recording;
                        })
                        .await;
                    });
                });
            }
        }
    }

    pub fn show_main_window() {
        let _ = std::process::Command::new("xdotool")
            .args(&["search", "--name", "Wflow", "windowactivate"])
            .status();
    }

    pub fn hide_main_window() {
        let _ = std::process::Command::new("xdotool")
            .args(&["search", "--name", "Wflow", "windowminimize"])
            .status();
    }

    #[allow(dead_code)]
    pub fn toggle_main_window() {
        // Handled via ToggleWindow event in coordinator
    }

    /// Checks if Wflow is configured to automatically start on Ubuntu/Linux boot via XDG Autostart
    pub fn is_autostart_enabled() -> bool {
        if let Ok(home) = std::env::var("HOME") {
            let desktop_path = std::path::PathBuf::from(home)
                .join(".config")
                .join("autostart")
                .join("wflow.desktop");
            return desktop_path.exists();
        }
        false
    }

    /// Enables or disables autostart on Ubuntu/Linux boot via XDG Autostart (.config/autostart/wflow.desktop)
    pub fn set_autostart(enable: bool) -> Result<()> {
        let home = std::env::var("HOME")?;
        let autostart_dir = std::path::PathBuf::from(home).join(".config").join("autostart");
        let desktop_file = autostart_dir.join("wflow.desktop");

        if enable {
            std::fs::create_dir_all(&autostart_dir)?;
            let exe_path = std::env::current_exe()?;
            let content = format!(
                "[Desktop Entry]\n\
                 Type=Application\n\
                 Name=Wflow\n\
                 Comment=High-performance native Rust AI voice dictation\n\
                 Exec=\"{}\" --startup\n\
                 Icon=wflow\n\
                 Terminal=false\n\
                 Categories=Utility;AudioVideo;Audio;\n\
                 X-GNOME-Autostart-enabled=true\n",
                exe_path.display()
            );
            std::fs::write(&desktop_file, content)?;
            info!("Enabled Linux XDG autostart at {:?}", desktop_file);
        } else {
            if desktop_file.exists() {
                let _ = std::fs::remove_file(&desktop_file);
                info!("Disabled Linux XDG autostart");
            }
        }
        Ok(())
    }
}

use anyhow::Result;
use std::sync::Mutex;
use std::thread::JoinHandle;
use tokio::sync::mpsc;
use tracing::info;

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
pub enum HotkeyEvent {
    Pressed,
    Released,
    ToggleWindow,
    QuitApp,
    KeyRecorded(u8, u16),
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
        initial_hotkey_vk: u16,
        initial_toggle_vk: u16,
    ) -> Result<JoinHandle<()>> {
        if let Ok(mut lock) = HOTKEY_VK.lock() {
            *lock = initial_hotkey_vk;
        }
        if let Ok(mut lock) = TOGGLE_VK.lock() {
            *lock = initial_toggle_vk;
        }

        info!("Starting Linux background hotkey listener thread...");
        let handle = std::thread::spawn(move || {
            while !tx.is_closed() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });

        Ok(handle)
    }

    pub fn get_hotkey_vk() -> u16 {
        *HOTKEY_VK.lock().unwrap()
    }

    pub fn get_toggle_window_vk() -> u16 {
        *TOGGLE_VK.lock().unwrap()
    }

    pub fn update_hotkeys(hotkey_vk: u16, toggle_vk: u16) {
        if let Ok(mut lock) = HOTKEY_VK.lock() {
            *lock = hotkey_vk;
        }
        if let Ok(mut lock) = TOGGLE_VK.lock() {
            *lock = toggle_vk;
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

    pub fn show_main_window() {}
    pub fn hide_main_window() {}
    pub fn toggle_main_window() {}

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

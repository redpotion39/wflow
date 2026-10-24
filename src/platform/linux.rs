use anyhow::{Context, Result};
use std::sync::Mutex;
use std::thread::JoinHandle;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::config::KeyCombo;
use ksni::TrayMethods;

static TRAY_HANDLE: Mutex<Option<ksni::Handle<WflowTray>>> = Mutex::new(None);

/// True while the X11 global hotkey listener is running. The GUI only needs its
/// own in-window fallback when no global hook is available (for example under
/// Wayland); running both would toggle dictation twice per key press.
static GLOBAL_HOTKEYS_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

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
            for pixel in data.chunks_mut(4) {
                pixel.rotate_right(1); // RGBA -> ARGB network byte order
            }
            vec![ksni::Icon {
                width,
                height,
                data,
            }]
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
        let tx3 = self.tx.clone();
        let tx4 = self.tx.clone();
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
                    "หยุดบันทึกเสียง (Stop Dictation)".into()
                } else {
                    "เริ่มบันทึกเสียง (Start Dictation)".into()
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
                label: "ตรวจสอบอัปเดต (Check for Updates)".into(),
                icon_name: "emblem-system-software-update".into(),
                activate: Box::new(move |_| {
                    let _ = tx4.send(HotkeyEvent::CheckUpdate);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "ปิดโปรแกรม (Quit Wflow)".into(),
                icon_name: "application-exit".into(),
                activate: Box::new(move |_| {
                    // Ask the coordinator to shut down gracefully so the
                    // database is flushed and the log file is closed.
                    let _ = tx3.send(HotkeyEvent::QuitApp);
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
    CheckUpdate,
}

static RECORDING_MODE: Mutex<Option<u8>> = Mutex::new(None);
static HOTKEY_VK: Mutex<u16> = Mutex::new(0x77); // F8
static TOGGLE_VK: Mutex<u16> = Mutex::new(0x78); // F9
static HOTKEY_COMBO: Mutex<KeyCombo> = Mutex::new(KeyCombo::single(0x77));
static TOGGLE_COMBO: Mutex<KeyCombo> = Mutex::new(KeyCombo::single(0x78));

#[cfg(target_os = "linux")]
mod x11_keys {
    use super::*;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::ConnectionExt;

    /// Polls the X11 keyboard state, which is the only portable way to get both
    /// key-down and key-up events for push-to-talk on X11/XWayland.
    pub struct Keyboard {
        conn: x11rb::rust_connection::RustConnection,
        keysyms: Vec<(u32, u8)>,
        keycode_vk: Vec<(u8, u16)>,
        pub ctrl: Vec<u8>,
        pub alt: Vec<u8>,
        pub shift: Vec<u8>,
        pub win: Vec<u8>,
    }

    impl Keyboard {
        pub fn connect() -> anyhow::Result<Self> {
            let (conn, _screen) = x11rb::connect(None)?;
            let setup = conn.setup();
            let min = setup.min_keycode;
            let count = setup.max_keycode.saturating_sub(min) + 1;
            let reply = conn.get_keyboard_mapping(min, count)?.reply()?;
            let per = if count == 0 {
                0
            } else {
                reply.keysyms.len() / count as usize
            };

            let mut keysyms = Vec::new();
            let mut keycode_vk = Vec::new();
            for (index, chunk) in reply.keysyms.chunks(per.max(1)).enumerate() {
                if let Some(&first) = chunk.first() {
                    let keycode = min + index as u8;
                    keysyms.push((first, keycode));
                    if let Some(vk) = keysym_to_vk(first) {
                        keycode_vk.push((keycode, vk));
                    }
                }
            }

            let code_for = |sym: u32| keysyms.iter().find(|(s, _)| *s == sym).map(|(_, c)| *c);
            let ctrl = [0xFFE3u32, 0xFFE4]
                .iter()
                .filter_map(|s| code_for(*s))
                .collect();
            let alt = [0xFFE9u32, 0xFFEA]
                .iter()
                .filter_map(|s| code_for(*s))
                .collect();
            let shift = [0xFFE1u32, 0xFFE2]
                .iter()
                .filter_map(|s| code_for(*s))
                .collect();
            let win = [0xFFEBu32, 0xFFEC]
                .iter()
                .filter_map(|s| code_for(*s))
                .collect();

            Ok(Self {
                conn,
                keysyms,
                keycode_vk,
                ctrl,
                alt,
                shift,
                win,
            })
        }

        pub fn keymap(&self) -> Option<Vec<u8>> {
            self.conn
                .query_keymap()
                .ok()?
                .reply()
                .ok()
                .map(|r| r.keys.to_vec())
        }

        pub fn keycode_for_vk(&self, vk: u16) -> Option<u8> {
            let sym = vk_to_keysym(vk)?;
            self.keysyms
                .iter()
                .find(|(s, _)| *s == sym)
                .map(|(_, code)| *code)
        }

        pub fn vk_for_keycode(&self, keycode: u8) -> Option<u16> {
            self.keycode_vk
                .iter()
                .find(|(code, _)| *code == keycode)
                .map(|(_, vk)| *vk)
        }
    }

    fn keysym_to_vk(sym: u32) -> Option<u16> {
        match sym {
            0x08 => Some(0x08),                                    // BackSpace
            0x09 => Some(0x09),                                    // Tab
            0xFF0D => Some(0x0D),                                  // Return
            0xFF1B => Some(0x1B),                                  // Escape
            0x20 => Some(0x20),                                    // Space
            0x30..=0x39 => Some(sym as u16),                       // 0-9
            0x61..=0x7A => Some(sym as u16 - 0x20),                // a-z -> VK A-Z
            0xFFBE..=0xFFC9 => Some(0x70 + (sym as u16 - 0xFFBE)), // F1-F12
            0xFFE1 | 0xFFE2 => Some(0x10),
            0xFFE3 | 0xFFE4 => Some(0x11),
            0xFFE9 | 0xFFEA => Some(0x12),
            0xFFEB | 0xFFEC => Some(0x5B),
            _ => None,
        }
    }

    fn vk_to_keysym(vk: u16) -> Option<u32> {
        match vk {
            0x08 => Some(0x08),
            0x09 => Some(0x09),
            0x0D => Some(0xFF0D),
            0x1B => Some(0xFF1B),
            0x20 => Some(0x20),
            0x30..=0x39 => Some(vk as u32),
            0x41..=0x5A => Some(vk as u32 + 0x20),
            0x70..=0x7B => Some(0xFFBE + (vk as u32 - 0x70)),
            0xA0..=0xA1 => Some(0xFFE1),
            0xA2..=0xA3 => Some(0xFFE3),
            0xA4..=0xA5 => Some(0xFFE9),
            0x5B..=0x5C => Some(0xFFEB),
            _ => None,
        }
    }

    /// `QueryKeymap` returns a 256-bit bitmap (32 bytes) in which bit `k` is
    /// set while keycode `k` is held.
    ///
    /// The keycode is therefore a **bit** index, not a **byte** index. Using
    /// it to index the byte slice directly only covers keycodes 0-31 and
    /// silently reports every real key (F8 is keycode 74) as "not pressed".
    pub fn is_down(keys: &[u8], code: u8) -> bool {
        match keys.get((code / 8) as usize) {
            Some(byte) => byte & (1 << (code % 8)) != 0,
            None => false,
        }
    }

    pub fn any_down(keys: &[u8], codes: &[u8]) -> bool {
        codes.iter().any(|code| is_down(keys, *code))
    }

    pub fn modifiers_match(keys: &[u8], kb: &Keyboard, combo: &KeyCombo) -> bool {
        any_down(keys, &kb.ctrl) == combo.ctrl
            && any_down(keys, &kb.alt) == combo.alt
            && any_down(keys, &kb.shift) == combo.shift
            && any_down(keys, &kb.win) == combo.win
    }
}

#[cfg(target_os = "linux")]
fn run_hotkey_loop(tx: &mpsc::UnboundedSender<HotkeyEvent>) -> anyhow::Result<()> {
    let keyboard = x11_keys::Keyboard::connect()?;
    GLOBAL_HOTKEYS_ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
    info!("Linux X11 global hotkey listener active");

    let mut prev_hot = false;
    let mut prev_toggle = false;
    let mut prev_keys = keyboard.keymap().unwrap_or_default();
    let mut prev_recording: Option<u8> = None;

    loop {
        if tx.is_closed() {
            return Ok(());
        }

        let Some(keys) = keyboard.keymap() else {
            std::thread::sleep(std::time::Duration::from_millis(100));
            continue;
        };

        let recording = *RECORDING_MODE.lock().unwrap_or_else(|e| e.into_inner());

        if recording.is_none() {
            let hotkey = *HOTKEY_COMBO.lock().unwrap_or_else(|e| e.into_inner());
            let toggle = *TOGGLE_COMBO.lock().unwrap_or_else(|e| e.into_inner());

            let hot = keyboard
                .keycode_for_vk(hotkey.key)
                .map(|code| x11_keys::is_down(&keys, code))
                .unwrap_or(false)
                && x11_keys::modifiers_match(&keys, &keyboard, &hotkey);

            if hot != prev_hot {
                tracing::debug!("Global dictation hotkey transition: down={hot}");
                let event = if hot {
                    HotkeyEvent::Pressed
                } else {
                    HotkeyEvent::Released
                };
                let _ = tx.send(event);
                prev_hot = hot;
            }

            let toggle_down = keyboard
                .keycode_for_vk(toggle.key)
                .map(|code| x11_keys::is_down(&keys, code))
                .unwrap_or(false)
                && x11_keys::modifiers_match(&keys, &keyboard, &toggle);

            if toggle_down && !prev_toggle {
                let _ = tx.send(HotkeyEvent::ToggleWindow);
            }
            prev_toggle = toggle_down;
        } else {
            prev_hot = false;
            prev_toggle = false;
        }

        // Hotkey recording mode: report the first newly pressed non-modifier key.
        if let Some(mode) = recording {
            let is_modifier = |vk: u16| matches!(vk, 0x10..=0x12 | 0xA0..=0xA5 | 0x5B | 0x5C);

            // The bitmap is indexed by keycode bit, so walk bits instead of
            // bytes and map each set bit back to its keycode.
            let mut newly_pressed = None;
            'scan: for (byte_index, byte) in keys.iter().enumerate() {
                for bit in 0..8u16 {
                    let code = byte_index as u16 * 8 + bit;
                    if code > u8::MAX as u16 {
                        break 'scan;
                    }
                    let down = byte & (1 << bit) != 0;
                    let was_down = prev_keys
                        .get(byte_index)
                        .map(|b| b & (1 << bit) != 0)
                        .unwrap_or(false);
                    if !down || was_down {
                        continue;
                    }
                    let Some(vk) = keyboard.vk_for_keycode(code as u8) else {
                        continue;
                    };
                    if is_modifier(vk) {
                        continue;
                    }
                    newly_pressed = Some(vk);
                    break 'scan;
                }
            }

            if let Some(vk) = newly_pressed {
                let combo = KeyCombo {
                    ctrl: x11_keys::any_down(&keys, &keyboard.ctrl),
                    alt: x11_keys::any_down(&keys, &keyboard.alt),
                    shift: x11_keys::any_down(&keys, &keyboard.shift),
                    win: x11_keys::any_down(&keys, &keyboard.win),
                    key: vk,
                };
                let _ = tx.send(HotkeyEvent::KeyRecorded(mode, combo));
                if let Ok(mut lock) = RECORDING_MODE.lock() {
                    *lock = None;
                }
            }
        }

        if prev_recording.is_some() && recording.is_none() {
            // Finished recording; start from a clean slate so the newly assigned
            // hotkey does not fire immediately.
            prev_hot = true;
        }
        prev_recording = recording;
        prev_keys = keys;

        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

pub struct PlatformManager;

impl PlatformManager {
    pub fn get_active_app_context() -> ActiveAppContext {
        // Try reading window title on Linux via xdotool if available
        let title_output = std::process::Command::new("xdotool")
            .args(["getactivewindow", "getwindowname"])
            .output();

        let window_title = match title_output {
            Ok(out) if out.status.success() => {
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            }
            _ => String::new(),
        };

        let pid_output = std::process::Command::new("xdotool")
            .args(["getactivewindow", "getwindowpid"])
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
            .args(["key", "ctrl+v"])
            .status();

        Ok(())
    }

    pub fn start_hotkey_listener(
        tx: mpsc::UnboundedSender<HotkeyEvent>,
        initial_hotkey: KeyCombo,
        initial_toggle: KeyCombo,
    ) -> Result<JoinHandle<()>> {
        if let Ok(mut lock) = HOTKEY_COMBO.lock() {
            *lock = initial_hotkey;
        }
        if let Ok(mut lock) = TOGGLE_COMBO.lock() {
            *lock = initial_toggle;
        }
        if let Ok(mut lock) = HOTKEY_VK.lock() {
            *lock = initial_hotkey.key;
        }
        if let Ok(mut lock) = TOGGLE_VK.lock() {
            *lock = initial_toggle.key;
        }

        // Spawn Linux System Tray via ksni in a background thread
        let tray_tx = tx.clone();
        let tray = WflowTray {
            is_recording: false,
            tx: tray_tx,
        };

        std::thread::spawn(move || match async_io::block_on(tray.spawn()) {
            Ok(handle) => {
                info!("Linux StatusNotifierItem System Tray started successfully!");
                if let Ok(mut lock) = TRAY_HANDLE.lock() {
                    *lock = Some(handle);
                }
            }
            Err(err) => {
                warn!(
                    "Could not start Linux StatusNotifierItem System Tray: {:?}",
                    err
                );
            }
        });

        info!("Starting Linux X11 background hotkey listener thread...");
        let handle = std::thread::spawn(move || {
            if let Err(err) = run_hotkey_loop(&tx) {
                warn!(
                    "Linux global hotkeys are unavailable ({}). Use the in-window dictation button or set DISPLAY for X11/XWayland.",
                    err
                );
            }
            GLOBAL_HOTKEYS_ACTIVE.store(false, std::sync::atomic::Ordering::SeqCst);
            while !tx.is_closed() {
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        });

        Ok(handle)
    }

    /// Whether a system-wide hotkey hook is active on this session.
    pub fn has_global_hotkeys() -> bool {
        GLOBAL_HOTKEYS_ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
    }

    #[allow(dead_code)]
    pub fn get_hotkey_vk() -> u16 {
        *HOTKEY_VK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[allow(dead_code)]
    pub fn get_toggle_window_vk() -> u16 {
        *TOGGLE_VK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[allow(dead_code)]
    pub fn get_hotkey_combo() -> KeyCombo {
        *HOTKEY_COMBO.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[allow(dead_code)]
    pub fn get_toggle_window_combo() -> KeyCombo {
        *TOGGLE_COMBO.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn update_hotkeys(hotkey: KeyCombo, toggle: KeyCombo) {
        if let Ok(mut lock) = HOTKEY_VK.lock() {
            *lock = hotkey.key;
        }
        if let Ok(mut lock) = TOGGLE_VK.lock() {
            *lock = toggle.key;
        }
        if let Ok(mut lock) = HOTKEY_COMBO.lock() {
            *lock = hotkey;
        }
        if let Ok(mut lock) = TOGGLE_COMBO.lock() {
            *lock = toggle;
        }
    }

    /// Opens a URL in the user's default browser / file manager.
    pub fn open_url(url: &str) -> Result<()> {
        std::process::Command::new("xdg-open")
            .arg(url)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .with_context(|| format!("Failed to open {}", url))
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
            .args(["search", "--name", "Wflow", "windowactivate"])
            .status();
    }

    pub fn hide_main_window() {
        let _ = std::process::Command::new("xdotool")
            .args(["search", "--name", "Wflow", "windowminimize"])
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
        let autostart_dir = std::path::PathBuf::from(home)
            .join(".config")
            .join("autostart");
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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::x11_keys::is_down;

    /// `QueryKeymap` is a 256-bit bitmap: keycode 74 (F8) lives in byte 9,
    /// bit 2. Indexing the byte slice by keycode used to read past the end.
    #[test]
    fn keymap_bits_are_indexed_by_keycode() {
        let mut keys = vec![0u8; 32];
        // Nothing is held in an all-zero bitmap, even for high keycodes.
        assert!(!is_down(&keys, 74));
        assert!(!is_down(&keys, 37));

        // Press F8 (keycode 74): byte 9, bit 2.
        keys[74 / 8] |= 1 << (74 % 8);
        assert!(is_down(&keys, 74));
        assert!(!is_down(&keys, 75));
        assert!(!is_down(&keys, 66));

        // Press Left Control (keycode 37): byte 4, bit 5.
        keys[37 / 8] |= 1 << (37 % 8);
        assert!(is_down(&keys, 37));
        assert!(!is_down(&keys, 38));
    }

    #[test]
    fn modifiers_resolve_to_real_keycodes() {
        // A combo that requires Control must not match a plain key press.
        let mut keys = vec![0u8; 32];
        keys[74 / 8] |= 1 << (74 % 8);
        assert!(is_down(&keys, 74));
        // Control is not held.
        assert!(!is_down(&keys, 37));
    }
}

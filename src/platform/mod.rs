#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "windows")]
pub use windows::*;

#[cfg(not(target_os = "windows"))]
pub mod mock {
    use anyhow::Result;
    use tokio::sync::mpsc;

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
        format!("Key(0x{:02X})", vk)
    }

    pub struct PlatformManager;

    impl PlatformManager {
        pub fn get_active_app_context() -> ActiveAppContext {
            ActiveAppContext::default()
        }

        pub fn paste_text(text: &str) -> Result<()> {
            println!("Simulated paste: {}", text);
            Ok(())
        }

        pub fn start_hotkey_listener(
            _tx: mpsc::UnboundedSender<HotkeyEvent>,
            _initial_hotkey_vk: u16,
            _initial_toggle_vk: u16,
        ) -> Result<std::thread::JoinHandle<()>> {
            Ok(std::thread::spawn(|| {}))
        }

        pub fn get_hotkey_vk() -> u16 {
            0x77
        }

        pub fn get_toggle_window_vk() -> u16 {
            0x78
        }

        pub fn update_hotkeys(_hotkey_vk: u16, _toggle_vk: u16) {}
        pub fn start_recording_hotkey(_mode: u8) {}
        pub fn stop_recording_hotkey() {}
        pub fn show_floating_hud(_badge: &str) {}
        pub fn set_hud_status(_status: crate::ui::DictationStatus) {}
        pub fn update_hud_amplitude(_amp: f32) {}
        pub fn hide_floating_hud() {}
        pub fn is_autostart_enabled() -> bool { false }
        pub fn set_autostart(_enable: bool) -> Result<()> { Ok(()) }
    }
}

#[cfg(not(target_os = "windows"))]
pub use mock::*;

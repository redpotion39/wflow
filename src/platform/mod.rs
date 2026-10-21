//! Platform abstraction.
//!
//! * Windows — native `WH_KEYBOARD_LL` hook, UI Automation, `SendInput` and a
//!   topmost HUD window.
//! * Linux — X11/XWayland global hotkeys, a StatusNotifierItem tray and
//!   `xdotool` based pasting.
//! * Anything else — a stub implementation that keeps the crate compiling.
//!
//! The stub is always type-checked, even on the platforms that do not use it,
//! so it cannot silently rot. `cargo check --features mock_check` additionally
//! routes the whole crate through the stub, which is how CI verifies the
//! macOS/other build.

mod keys;
pub use keys::vk_to_string;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "windows")]
pub use windows::*;

#[cfg(all(target_os = "linux", not(feature = "mock_check")))]
pub mod linux;

#[cfg(all(target_os = "linux", not(feature = "mock_check")))]
pub use linux::*;

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
        KeyRecorded(u8, crate::config::KeyCombo),
        AutostartToggled(bool),
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
            _initial_hotkey: crate::config::KeyCombo,
            _initial_toggle: crate::config::KeyCombo,
        ) -> Result<std::thread::JoinHandle<()>> {
            Ok(std::thread::spawn(|| {}))
        }

        pub fn get_hotkey_vk() -> u16 {
            0x77
        }

        pub fn get_toggle_window_vk() -> u16 {
            0x78
        }

        pub fn get_hotkey_combo() -> crate::config::KeyCombo {
            crate::config::KeyCombo::single(0x77)
        }

        pub fn get_toggle_window_combo() -> crate::config::KeyCombo {
            crate::config::KeyCombo::single(0x78)
        }

        pub fn update_hotkeys(_hotkey: crate::config::KeyCombo, _toggle: crate::config::KeyCombo) {}
        pub fn start_recording_hotkey(_mode: u8) {}
        pub fn stop_recording_hotkey() {}
        pub fn show_floating_hud(_badge: &str) {}
        pub fn set_hud_status(_status: crate::ui::DictationStatus) {}
        pub fn update_hud_amplitude(_amp: f32) {}
        pub fn hide_floating_hud() {}
        pub fn set_tray_recording_state(_is_recording: bool) {}
        pub fn update_tray_tooltip(
            _hotkey: crate::config::KeyCombo,
            _toggle: crate::config::KeyCombo,
        ) {
        }
        pub fn show_main_window() {}
        pub fn hide_main_window() {}
        pub fn toggle_main_window() {}
        pub fn is_autostart_enabled() -> bool {
            false
        }
        pub fn set_autostart(_enable: bool) -> Result<()> {
            Ok(())
        }
        pub fn open_url(_url: &str) -> Result<()> {
            Ok(())
        }
    }
}

#[cfg(any(
    all(not(target_os = "windows"), not(target_os = "linux")),
    feature = "mock_check"
))]
pub use mock::*;

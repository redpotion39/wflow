use anyhow::{Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use windows::core::Interface;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateBitmap, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW,
    CreatePen, CreateRoundRectRgn, CreateSolidBrush, DeleteDC, DeleteObject, EndPaint,
    InvalidateRect, RoundRect, SelectObject, SetBkMode, SetTextColor, SetWindowRgn, TextOutW,
    FW_BOLD, FW_NORMAL, PAINTSTRUCT, PS_NULL, PS_SOLID, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::ProcessStatus::K32GetModuleFileNameExW;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationValuePattern, UIA_ValuePatternId,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VK_CONTROL, VK_V,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CallNextHookEx, CreateIconIndirect, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyMenu, DispatchMessageW, EnumWindows, FindWindowW, GetClassNameW,
    GetClientRect, GetCursorPos, GetForegroundWindow, GetMessageW, GetSystemMetrics,
    GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, KillTimer, LoadIconW, LoadImageW,
    PostQuitMessage, RegisterClassW, SendMessageW, SetForegroundWindow, SetTimer, SetWindowPos,
    SetWindowsHookExW, ShowWindow, TrackPopupMenu, TranslateMessage, UnhookWindowsHookEx, HICON,
    HWND_TOP, ICONINFO, IDI_APPLICATION, IMAGE_ICON, KBDLLHOOKSTRUCT, LR_DEFAULTCOLOR, MF_CHECKED,
    MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, SM_CXSCREEN, SM_CXSMICON, SM_CYSCREEN, SM_CYSMICON,
    SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_HIDE, SW_RESTORE, SW_SHOW, SW_SHOWNOACTIVATE,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, WH_KEYBOARD_LL, WINDOW_EX_STYLE, WM_APP, WM_CLOSE,
    WM_ENDSESSION, WM_ERASEBKGND, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_PAINT,
    WM_QUERYENDSESSION, WM_RBUTTONUP, WM_SETICON, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_TIMER, WNDCLASSW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_OVERLAPPED, WS_POPUP,
};

use crate::ui::DictationStatus;

#[derive(Clone, Debug, Default)]
pub struct ActiveAppContext {
    pub process_name: String,
    pub window_title: String,
    pub before_text: Option<String>,
    pub after_text: Option<String>,
    pub selected_text: Option<String>,
    pub full_text: Option<String>,
}

use crate::config::KeyCombo;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Pressed,
    Released,
    #[allow(dead_code)]
    ToggleWindow,
    QuitApp,
    KeyRecorded(u8, KeyCombo), // mode (1=Dictate, 2=ToggleWindow), combo
    AutostartToggled(bool),
    CheckUpdate,
}

// Global hook callback state
static HOOK_TX: std::sync::Mutex<Option<mpsc::UnboundedSender<HotkeyEvent>>> =
    std::sync::Mutex::new(None);
static HOTKEY_DOWN: AtomicBool = AtomicBool::new(false);
static TOGGLE_DOWN: AtomicBool = AtomicBool::new(false);
static HOTKEY_COMBO: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x77); // Default F8 (single)
static TOGGLE_WINDOW_COMBO: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x78); // Default F9 (single)
static RECORDING_MODE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0); // 0=None, 1=Dictate, 2=ToggleWindow
static RECORDING_NON_MOD_PRESSED: AtomicBool = AtomicBool::new(false);
static TRAY_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static HUD_HWND: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static HUD_STATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0); // 0=Hidden, 1=Listening, 2=Transcribing, 3=Done
static HUD_AMPLITUDE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static HUD_TICK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static HUD_BADGE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
const WM_TRAY_CALLBACK: u32 = WM_APP + 101;

pub fn is_key_down(vk: i32) -> bool {
    unsafe {
        (windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(vk) as u16 & 0x8000) != 0
    }
}

pub fn is_modifier_key(vk: u16) -> bool {
    matches!(
        vk,
        0x10 | 0xA0 | 0xA1 // Shift
        | 0x11 | 0xA2 | 0xA3 // Ctrl
        | 0x12 | 0xA4 | 0xA5 // Alt
        | 0x5B | 0x5C // Win
    )
}

pub fn current_ctrl_pressed() -> bool {
    is_key_down(0x11) || is_key_down(0xA2) || is_key_down(0xA3)
}

pub fn current_alt_pressed() -> bool {
    is_key_down(0x12) || is_key_down(0xA4) || is_key_down(0xA5)
}

pub fn current_shift_pressed() -> bool {
    is_key_down(0x10) || is_key_down(0xA0) || is_key_down(0xA1)
}

pub fn current_win_pressed() -> bool {
    is_key_down(0x5B) || is_key_down(0x5C)
}

pub fn matches_vk(event_vk: u16, target_vk: u16) -> bool {
    if event_vk == target_vk {
        return true;
    }
    if target_vk == 0x11 && (event_vk == 0xA2 || event_vk == 0xA3) {
        return true;
    }
    if target_vk == 0x10 && (event_vk == 0xA0 || event_vk == 0xA1) {
        return true;
    }
    if target_vk == 0x12 && (event_vk == 0xA4 || event_vk == 0xA5) {
        return true;
    }
    false
}

pub struct PlatformManager;

impl PlatformManager {
    /// Captures the currently active window and focused text box details using UI Automation
    pub fn get_active_app_context() -> ActiveAppContext {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        let mut ctx = ActiveAppContext::default();

        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.0 == 0 as _ {
                return ctx;
            }

            // Window title
            let mut title_buf = [0u16; 512];
            let len = GetWindowTextW(hwnd, &mut title_buf);
            if len > 0 {
                ctx.window_title = String::from_utf16_lossy(&title_buf[..len as usize]);
            }

            // Process name
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid > 0 {
                if let Ok(handle) =
                    OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid)
                {
                    let mut proc_buf = [0u16; 1024];
                    let proc_len = K32GetModuleFileNameExW(handle, None, &mut proc_buf);
                    if proc_len > 0 {
                        let full_path = String::from_utf16_lossy(&proc_buf[..proc_len as usize]);
                        ctx.process_name = full_path
                            .rsplit(['\\', '/'])
                            .next()
                            .unwrap_or(&full_path)
                            .to_string();
                    }
                    let _ = windows::Win32::Foundation::CloseHandle(handle);
                }
            }

            // UI Automation for focused textbox
            if let Ok(automation) =
                CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER)
            {
                if let Ok(focused) = automation.GetFocusedElement() {
                    if let Ok(pattern_obj) = focused.GetCurrentPattern(UIA_ValuePatternId) {
                        if let Ok(value_pattern) = pattern_obj.cast::<IUIAutomationValuePattern>() {
                            if let Ok(val) = value_pattern.CurrentValue() {
                                let text = val.to_string();
                                if !text.is_empty() {
                                    ctx.full_text = Some(text.clone());
                                    ctx.before_text = Some(text);
                                }
                            }
                        }
                    }
                }
            }

            CoUninitialize();
        }

        ctx
    }

    /// Opens a URL or local folder in the user's default handler.
    pub fn open_url(url: &str) -> Result<()> {
        use windows::core::PCWSTR;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let wide: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
        let operation: Vec<u16> = "open\0".encode_utf16().collect();
        unsafe {
            let result = ShellExecuteW(
                HWND(std::ptr::null_mut()),
                PCWSTR(operation.as_ptr()),
                PCWSTR(wide.as_ptr()),
                None::<&PCWSTR>,
                None::<&PCWSTR>,
                SW_SHOWNORMAL,
            );
            if result.0 as isize <= 32 {
                anyhow::bail!("ShellExecuteW failed for {}", url);
            }
        }
        Ok(())
    }

    /// Simulates pasting the given text into the currently focused window
    pub fn paste_text(text: &str) -> Result<()> {
        info!("Inserting text: '{}'", text);

        // 1. Copy text to clipboard via arboard
        let mut clipboard =
            arboard::Clipboard::new().context("Failed to access system clipboard")?;
        let prev_text = clipboard.get_text().ok();

        clipboard
            .set_text(text)
            .context("Failed to write text to clipboard")?;

        // 2. Small delay to ensure clipboard is populated
        std::thread::sleep(std::time::Duration::from_millis(20));

        // 3. Send Ctrl + V
        unsafe {
            let inputs = [
                // Ctrl down
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_CONTROL,
                            wScan: 0,
                            dwFlags: KEYBD_EVENT_FLAGS(0),
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
                // V down
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_V,
                            wScan: 0,
                            dwFlags: KEYBD_EVENT_FLAGS(0),
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
                // V up
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_V,
                            wScan: 0,
                            dwFlags: KEYEVENTF_KEYUP,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
                // Ctrl up
                INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: VK_CONTROL,
                            wScan: 0,
                            dwFlags: KEYEVENTF_KEYUP,
                            time: 0,
                            dwExtraInfo: 0,
                        },
                    },
                },
            ];

            let sent = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
            if sent != inputs.len() as u32 {
                warn!("SendInput only sent {}/{} events", sent, inputs.len());
            }
        }

        // 4. Restore previous clipboard content after short delay
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(500));
            if let Some(prev) = prev_text {
                if let Ok(mut cb) = arboard::Clipboard::new() {
                    let _ = cb.set_text(prev);
                }
            }
        });

        Ok(())
    }

    /// Spawns a background low-level keyboard hook monitoring push-to-talk hotkey and manages System Tray icon
    pub fn start_hotkey_listener(
        tx: mpsc::UnboundedSender<HotkeyEvent>,
        initial_hotkey: KeyCombo,
        initial_toggle: KeyCombo,
    ) -> Result<std::thread::JoinHandle<()>> {
        HOTKEY_COMBO.store(initial_hotkey.to_u32(), Ordering::SeqCst);
        TOGGLE_WINDOW_COMBO.store(initial_toggle.to_u32(), Ordering::SeqCst);

        let handle = std::thread::spawn(move || unsafe {
            if let Ok(mut lock) = HOOK_TX.lock() {
                *lock = Some(tx);
            }

            let class_name = windows::core::w!("WisprFlowTrayMsgClass");
            let wndclass = WNDCLASSW {
                lpfnWndProc: Some(tray_wnd_proc),
                lpszClassName: windows::core::PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            let _ = RegisterClassW(&wndclass);

            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::PCWSTR(class_name.as_ptr()),
                windows::core::w!("WisprFlowTrayMsgWindow"),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                None,
                None,
            )
            .unwrap_or_default();

            TRAY_HWND.store(hwnd.0 as isize, Ordering::SeqCst);

            // Register and create floating HUD overlay window
            let hud_class_name = windows::core::w!("WisprFlowFloatingHudClass");
            let hud_wndclass = WNDCLASSW {
                lpfnWndProc: Some(hud_wnd_proc),
                lpszClassName: windows::core::PCWSTR(hud_class_name.as_ptr()),
                ..Default::default()
            };
            let _ = RegisterClassW(&hud_wndclass);

            let screen_w = GetSystemMetrics(SM_CXSCREEN);
            let screen_h = GetSystemMetrics(SM_CYSCREEN);
            let hud_w = 280;
            let hud_h = 58;
            let hud_x = (screen_w - hud_w) / 2;
            let hud_y = screen_h - hud_h - 75;

            let hud_hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                windows::core::PCWSTR(hud_class_name.as_ptr()),
                windows::core::w!("WisprFlowFloatingHudWindow"),
                WS_POPUP,
                hud_x,
                hud_y,
                hud_w,
                hud_h,
                None,
                None,
                None,
                None,
            )
            .unwrap_or_default();

            let rgn = CreateRoundRectRgn(0, 0, hud_w + 1, hud_h + 1, 28, 28);
            let _ = SetWindowRgn(hud_hwnd, rgn, true);

            HUD_HWND.store(hud_hwnd.0 as isize, Ordering::SeqCst);

            let mut nid = NOTIFYICONDATAW::default();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = hwnd;
            nid.uID = 1001;
            nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.uCallbackMessage = WM_TRAY_CALLBACK;
            nid.hIcon = Self::get_app_icon(1);

            let tip = format!(
                "Wflow\n{}: Dictate | {}: Toggle Window\0",
                initial_hotkey.format_display(),
                initial_toggle.format_display()
            );
            for (i, ch) in tip.encode_utf16().enumerate() {
                if i < nid.szTip.len() - 1 {
                    nid.szTip[i] = ch;
                }
            }
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
            info!(
                "System Tray icon registered in Windows Taskbar ({}: Dictate, {}: Toggle Window).",
                initial_hotkey.format_display(),
                initial_toggle.format_display()
            );

            let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(low_level_keyboard_proc), None, 0);

            if let Ok(h) = hook {
                info!(
                    "Global keyboard hook registered (Dictate: {}, Toggle: {}).",
                    initial_hotkey.format_display(),
                    initial_toggle.format_display()
                );

                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }

                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
                let _ = UnhookWindowsHookEx(h);
            } else {
                error!("Failed to register low-level keyboard hook");
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            }

            HUD_HWND.store(0, Ordering::SeqCst);
            TRAY_HWND.store(0, Ordering::SeqCst);
        });

        Ok(handle)
    }

    #[allow(dead_code)]
    pub fn get_hotkey_vk() -> u16 {
        Self::get_hotkey_combo().key
    }

    #[allow(dead_code)]
    pub fn get_toggle_window_vk() -> u16 {
        Self::get_toggle_window_combo().key
    }

    pub fn get_hotkey_combo() -> KeyCombo {
        KeyCombo::from_u32(HOTKEY_COMBO.load(Ordering::SeqCst))
    }

    pub fn get_toggle_window_combo() -> KeyCombo {
        KeyCombo::from_u32(TOGGLE_WINDOW_COMBO.load(Ordering::SeqCst))
    }

    pub fn update_hotkeys(hotkey: KeyCombo, toggle: KeyCombo) {
        HOTKEY_COMBO.store(hotkey.to_u32(), Ordering::SeqCst);
        TOGGLE_WINDOW_COMBO.store(toggle.to_u32(), Ordering::SeqCst);
        info!(
            "Updated active hotkeys: Dictate = {}, Toggle = {}",
            hotkey.format_display(),
            toggle.format_display(),
        );
        Self::update_tray_tooltip(hotkey, toggle);
    }

    pub fn start_recording_hotkey(mode: u8) {
        RECORDING_NON_MOD_PRESSED.store(false, Ordering::SeqCst);
        RECORDING_MODE.store(mode, Ordering::SeqCst);
        info!("Hotkey recording mode set to: {}", mode);
    }

    pub fn stop_recording_hotkey() {
        RECORDING_MODE.store(0, Ordering::SeqCst);
        info!("Hotkey recording mode stopped.");
    }

    pub fn show_floating_hud(provider_badge: &str) {
        if let Ok(mut lock) = HUD_BADGE.lock() {
            *lock = provider_badge.to_string();
        }
        HUD_STATE.store(1, Ordering::SeqCst); // 1 = Listening
        let val = HUD_HWND.load(Ordering::SeqCst);
        if val != 0 {
            let hwnd = HWND(val as *mut core::ffi::c_void);
            unsafe {
                let _ = SetTimer(hwnd, 1, 30, None);
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
                let _ = InvalidateRect(hwnd, None, false);
            }
        }
    }

    /// Converts embedded PNG bytes directly into a native Win32 32-bit ARGB HICON with alpha
    pub fn create_hicon_from_png_bytes(png_bytes: &[u8]) -> Option<HICON> {
        let img = image::load_from_memory(png_bytes).ok()?.into_rgba8();
        let (width, height) = img.dimensions();

        let mut bgra = Vec::with_capacity((width * height * 4) as usize);
        for pixel in img.pixels() {
            bgra.push(pixel[2]); // B
            bgra.push(pixel[1]); // G
            bgra.push(pixel[0]); // R
            bgra.push(pixel[3]); // A
        }

        unsafe {
            let hbm_color = CreateBitmap(
                width as i32,
                height as i32,
                1,
                32,
                Some(bgra.as_ptr() as *const _),
            );
            if hbm_color.is_invalid() {
                return None;
            }

            let mask_bytes_len = (((width + 15) / 16) * 2 * height) as usize;
            let mask_data = vec![0u8; mask_bytes_len];
            let hbm_mask = CreateBitmap(
                width as i32,
                height as i32,
                1,
                1,
                Some(mask_data.as_ptr() as *const _),
            );
            if hbm_mask.is_invalid() {
                let _ = DeleteObject(hbm_color);
                return None;
            }

            let mut icon_info = ICONINFO {
                fIcon: true.into(),
                xHotspot: 0,
                yHotspot: 0,
                hbmMask: hbm_mask,
                hbmColor: hbm_color,
            };

            let hicon = CreateIconIndirect(&mut icon_info).ok();

            let _ = DeleteObject(hbm_color);
            let _ = DeleteObject(hbm_mask);

            hicon
        }
    }

    /// Loads an icon embedded in the PE resource section (1: App Icon, 2: Recording Icon),
    /// with multi-resolution DPI scaling via LoadImageW, and guaranteed memory fallback via pure PNG decode.
    pub fn get_app_icon(resource_id: u16) -> HICON {
        unsafe {
            if let Ok(hmodule) = GetModuleHandleW(None) {
                let sm_cx = GetSystemMetrics(SM_CXSMICON);
                let sm_cy = GetSystemMetrics(SM_CYSMICON);
                if let Ok(handle) = LoadImageW(
                    hmodule,
                    windows::core::PCWSTR(resource_id as usize as *const u16),
                    IMAGE_ICON,
                    sm_cx,
                    sm_cy,
                    LR_DEFAULTCOLOR,
                ) {
                    let icon = HICON(handle.0);
                    if !icon.is_invalid() && icon.0 != std::ptr::null_mut() {
                        return icon;
                    }
                }
                if let Ok(icon) = LoadIconW(
                    hmodule,
                    windows::core::PCWSTR(resource_id as usize as *const u16),
                ) {
                    if !icon.is_invalid() && icon.0 != std::ptr::null_mut() {
                        return icon;
                    }
                }
            }

            // Guaranteed in-memory fallback using embedded crisp PNG:
            let png_bytes = if resource_id == 2 {
                include_bytes!("../../assets/tray_rec_32.png").as_slice()
            } else {
                include_bytes!("../../assets/tray_32.png").as_slice()
            };

            if let Some(icon) = Self::create_hicon_from_png_bytes(png_bytes) {
                return icon;
            }

            LoadIconW(None, IDI_APPLICATION).unwrap_or_default()
        }
    }

    /// Sets the native Win32 window icon for the main GUI window
    pub fn apply_window_icon(icon: HICON) {
        unsafe {
            if let Some(hwnd) = Self::find_main_window_hwnd() {
                let _ = SendMessageW(
                    hwnd,
                    WM_SETICON,
                    WPARAM(1), // ICON_BIG
                    LPARAM(icon.0 as isize),
                );
                let _ = SendMessageW(
                    hwnd,
                    WM_SETICON,
                    WPARAM(0), // ICON_SMALL
                    LPARAM(icon.0 as isize),
                );
            }
        }
    }

    /// Dynamically switches the Windows System Tray icon between idle and recording state
    pub fn set_tray_recording_state(is_recording: bool) {
        unsafe {
            let val = TRAY_HWND.load(Ordering::SeqCst);
            if val != 0 {
                let hwnd = HWND(val as *mut core::ffi::c_void);
                let mut nid = NOTIFYICONDATAW::default();
                nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
                nid.hWnd = hwnd;
                nid.uID = 1001;
                nid.uFlags = NIF_ICON;
                let icon_id = if is_recording { 2 } else { 1 };
                nid.hIcon = Self::get_app_icon(icon_id);
                let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
            }
        }
    }

    pub fn set_hud_status(status: DictationStatus) {
        Self::set_tray_recording_state(status == DictationStatus::Listening);

        let st_num = match status {
            DictationStatus::Listening => 1,
            DictationStatus::Transcribing => 2,
            DictationStatus::Done => 3,
            _ => 0,
        };
        HUD_STATE.store(st_num, Ordering::SeqCst);
        let val = HUD_HWND.load(Ordering::SeqCst);
        if val != 0 {
            let hwnd = HWND(val as *mut core::ffi::c_void);
            unsafe {
                let _ = InvalidateRect(hwnd, None, false);
            }
        }
    }

    pub fn update_hud_amplitude(amp: f32) {
        HUD_AMPLITUDE.store(amp.to_bits(), Ordering::SeqCst);
    }

    pub fn hide_floating_hud() {
        HUD_STATE.store(0, Ordering::SeqCst);
        let val = HUD_HWND.load(Ordering::SeqCst);
        if val != 0 {
            let hwnd = HWND(val as *mut core::ffi::c_void);
            unsafe {
                let _ = KillTimer(hwnd, 1);
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }

    pub fn update_tray_tooltip(hotkey: KeyCombo, toggle: KeyCombo) {
        unsafe {
            let val = TRAY_HWND.load(Ordering::SeqCst);
            if val != 0 {
                let hwnd = HWND(val as *mut core::ffi::c_void);
                let mut nid = NOTIFYICONDATAW::default();
                nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
                nid.hWnd = hwnd;
                nid.uID = 1001;
                nid.uFlags = NIF_TIP;

                let tip = format!(
                    "Wflow\n{}: Dictate | {}: Toggle Window\0",
                    hotkey.format_display(),
                    toggle.format_display(),
                );
                for (i, ch) in tip.encode_utf16().enumerate() {
                    if i < nid.szTip.len() - 1 {
                        nid.szTip[i] = ch;
                    }
                }
                let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
            }
        }
    }

    /// Toggles visibility of the main GUI window using direct Win32 APIs
    pub fn toggle_main_window() {
        unsafe {
            if let Some(hwnd) = Self::find_main_window_hwnd() {
                let is_vis = IsWindowVisible(hwnd).as_bool();
                if is_vis {
                    info!("Hiding GUI window to system tray (HWND {:?})", hwnd);
                    let _ = ShowWindow(hwnd, SW_HIDE);
                } else {
                    info!("Restoring GUI window from system tray (HWND {:?})", hwnd);
                    let _ = ShowWindow(hwnd, SW_SHOW);
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                    let _ = SetWindowPos(
                        hwnd,
                        HWND_TOP,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
                    );
                    let _ = SetForegroundWindow(hwnd);
                }
            } else {
                warn!("Could not find GUI window to toggle");
            }
        }
    }

    /// Shows and activates the main GUI window
    pub fn show_main_window() {
        unsafe {
            if let Some(hwnd) = Self::find_main_window_hwnd() {
                info!("Showing GUI window from system tray (HWND {:?})", hwnd);
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = ShowWindow(hwnd, SW_RESTORE);
                let _ = SetWindowPos(
                    hwnd,
                    HWND_TOP,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
                );
                let _ = SetForegroundWindow(hwnd);
            }
        }
    }

    /// Hides the main GUI window to system tray
    pub fn hide_main_window() {
        unsafe {
            if let Some(hwnd) = Self::find_main_window_hwnd() {
                info!("Hiding GUI window to system tray (HWND {:?})", hwnd);
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }

    /// Finds the HWND of the main GUI window
    pub fn find_main_window_hwnd() -> Option<HWND> {
        unsafe {
            // 1. Try FindWindowW by title
            let title = windows::core::w!("Wflow - AI Voice Studio");
            if let Ok(h) = FindWindowW(None, title) {
                if !h.is_invalid() && h.0 != std::ptr::null_mut() {
                    return Some(h);
                }
            }

            // 2. Enumerate all windows of this process
            let current_pid = windows::Win32::System::Threading::GetCurrentProcessId();
            struct EnumCtx {
                target_pid: u32,
                found: Option<HWND>,
            }
            let mut ctx = EnumCtx {
                target_pid: current_pid,
                found: None,
            };

            unsafe extern "system" fn enum_proc(
                hwnd: HWND,
                lparam: LPARAM,
            ) -> windows::Win32::Foundation::BOOL {
                let p = lparam.0 as *mut EnumCtx;
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid == (*p).target_pid {
                    let mut class_name = [0u16; 256];
                    let len = GetClassNameW(hwnd, &mut class_name);
                    let class_str = String::from_utf16_lossy(&class_name[..len as usize]);
                    if !class_str.contains("WisprFlowTrayMsg")
                        && !class_str.contains("WflowTrayMsg")
                    {
                        (*p).found = Some(hwnd);
                        return windows::Win32::Foundation::BOOL(0);
                    }
                }
                windows::Win32::Foundation::BOOL(1)
            }

            let _ = EnumWindows(Some(enum_proc), LPARAM(&mut ctx as *mut EnumCtx as isize));
            ctx.found
        }
    }

    /// Checks if Wflow is configured to automatically start on Windows boot
    pub fn is_autostart_enabled() -> bool {
        unsafe {
            let subkey = windows::core::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
            let mut hkey = HKEY::default();
            if RegOpenKeyExW(HKEY_CURRENT_USER, subkey, 0, KEY_QUERY_VALUE, &mut hkey).is_ok() {
                let value_name = windows::core::w!("Wflow");
                let mut data_len = 0u32;
                let query_res =
                    RegQueryValueExW(hkey, value_name, None, None, None, Some(&mut data_len));
                let _ = RegCloseKey(hkey);
                if query_res.is_ok() && data_len > 0 {
                    return true;
                }
            }
        }

        // Also check if shortcut exists in Startup folder
        if let Ok(app_data) = std::env::var("APPDATA") {
            let shortcut = std::path::PathBuf::from(app_data)
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs")
                .join("Startup")
                .join("Wflow.lnk");
            if shortcut.exists() {
                return true;
            }
        }

        false
    }

    /// Enables or disables autostart on Windows boot via Registry (and clears Startup shortcut if disabling)
    pub fn set_autostart(enable: bool) -> anyhow::Result<()> {
        unsafe {
            let subkey = windows::core::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
            let value_name = windows::core::w!("Wflow");
            let mut hkey = HKEY::default();

            if enable {
                let exe_path = std::env::current_exe()?;
                let val_str = format!("\"{}\" --startup\0", exe_path.to_string_lossy());
                let val_wide: Vec<u16> = val_str.encode_utf16().collect();
                let byte_slice =
                    std::slice::from_raw_parts(val_wide.as_ptr() as *const u8, val_wide.len() * 2);

                RegOpenKeyExW(HKEY_CURRENT_USER, subkey, 0, KEY_SET_VALUE, &mut hkey).ok()?;
                let res = RegSetValueExW(hkey, value_name, 0, REG_SZ, Some(byte_slice));
                let _ = RegCloseKey(hkey);
                res.ok()?;
                info!("Enabled Windows autostart: {:?}", val_str);
            } else {
                if RegOpenKeyExW(HKEY_CURRENT_USER, subkey, 0, KEY_SET_VALUE, &mut hkey).is_ok() {
                    let _ = RegDeleteValueW(hkey, value_name);
                    let _ = RegCloseKey(hkey);
                    info!("Disabled Windows autostart from registry");
                }

                if let Ok(app_data) = std::env::var("APPDATA") {
                    let shortcut = std::path::PathBuf::from(app_data)
                        .join("Microsoft")
                        .join("Windows")
                        .join("Start Menu")
                        .join("Programs")
                        .join("Startup")
                        .join("Wflow.lnk");
                    if shortcut.exists() {
                        let _ = std::fs::remove_file(shortcut);
                        info!("Removed Startup folder shortcut");
                    }
                }
            }
        }
        Ok(())
    }
}

unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if msg == WM_TRAY_CALLBACK {
        let event = (l_param.0 as u32) & 0xFFFF;
        info!("System tray mouse event: 0x{:04X}", event);
        if event == WM_LBUTTONUP || event == WM_LBUTTONDBLCLK {
            PlatformManager::toggle_main_window();
        } else if event == WM_RBUTTONUP {
            let mut pt = POINT::default();
            let _ = GetCursorPos(&mut pt);
            if let Ok(hmenu) = CreatePopupMenu() {
                let toggle_name = PlatformManager::get_toggle_window_combo().format_display();
                let open_str = format!("Open Wflow ({})\0", toggle_name);
                let open_wide: Vec<u16> = open_str.encode_utf16().collect();

                let _ = AppendMenuW(
                    hmenu,
                    MF_STRING,
                    1,
                    windows::core::PCWSTR(open_wide.as_ptr()),
                );
                let _ = AppendMenuW(hmenu, MF_STRING, 2, windows::core::w!("Hide to Tray"));
                let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, None);
                let _ = AppendMenuW(
                    hmenu,
                    MF_STRING,
                    5,
                    windows::core::w!("Check for Updates\u{2713}"),
                );

                let is_autostart = PlatformManager::is_autostart_enabled();
                let autostart_str = if is_autostart {
                    "Start Wflow on Windows Startup\0"
                } else {
                    "Start Wflow on Windows Startup\0"
                };
                let autostart_wide: Vec<u16> = autostart_str.encode_utf16().collect();
                let autostart_flags = MF_STRING
                    | if is_autostart {
                        MF_CHECKED
                    } else {
                        MF_UNCHECKED
                    };
                let _ = AppendMenuW(
                    hmenu,
                    autostart_flags,
                    4,
                    windows::core::PCWSTR(autostart_wide.as_ptr()),
                );

                let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, None);
                let _ = AppendMenuW(hmenu, MF_STRING, 3, windows::core::w!("Exit Application"));

                let _ = SetForegroundWindow(hwnd);
                let cmd = TrackPopupMenu(
                    hmenu,
                    TPM_RIGHTBUTTON | TPM_RETURNCMD,
                    pt.x,
                    pt.y,
                    0,
                    hwnd,
                    None,
                );
                let _ = DestroyMenu(hmenu);

                if cmd.0 == 1 {
                    PlatformManager::show_main_window();
                } else if cmd.0 == 2 {
                    PlatformManager::hide_main_window();
                } else if cmd.0 == 3 {
                    if let Ok(guard) = HOOK_TX.lock() {
                        if let Some(tx) = guard.as_ref() {
                            let _ = tx.send(HotkeyEvent::QuitApp);
                        }
                    }
                    let _ = PostQuitMessage(0);
                } else if cmd.0 == 5 {
                    if let Ok(guard) = HOOK_TX.lock() {
                        if let Some(tx) = guard.as_ref() {
                            let _ = tx.send(HotkeyEvent::CheckUpdate);
                        }
                    }
                } else if cmd.0 == 4 {
                    let new_state = !PlatformManager::is_autostart_enabled();
                    let _ = PlatformManager::set_autostart(new_state);
                    if let Ok(guard) = HOOK_TX.lock() {
                        if let Some(tx) = guard.as_ref() {
                            let _ = tx.send(HotkeyEvent::AutostartToggled(new_state));
                        }
                    }
                }
            }
        }
        return LRESULT(0);
    }
    if msg == WM_CLOSE || msg == WM_ENDSESSION {
        info!("Tray window received WM_CLOSE / WM_ENDSESSION, shutting down...");
        if let Ok(guard) = HOOK_TX.lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send(HotkeyEvent::QuitApp);
            }
        }
        let _ = PostQuitMessage(0);
        return LRESULT(0);
    }
    if msg == WM_QUERYENDSESSION {
        return LRESULT(1);
    }
    DefWindowProcW(hwnd, msg, w_param, l_param)
}

unsafe extern "system" fn low_level_keyboard_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if n_code >= 0 {
        let kbd_struct = *(l_param.0 as *const KBDLLHOOKSTRUCT);
        let vk_code = kbd_struct.vkCode as u16;
        let msg = w_param.0 as u32;
        let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;

        // 1. If currently in hotkey recording mode, capture the combination
        let rec_mode = RECORDING_MODE.load(Ordering::SeqCst);
        if rec_mode != 0 {
            if is_down {
                if vk_code == 0x1B
                /* Escape */
                {
                    RECORDING_MODE.store(0, Ordering::SeqCst);
                    if let Ok(guard) = HOOK_TX.lock() {
                        if let Some(tx) = guard.as_ref() {
                            let _ =
                                tx.send(HotkeyEvent::KeyRecorded(rec_mode, KeyCombo::single(0)));
                        }
                    }
                    return LRESULT(1); // Suppress Escape
                }

                if is_modifier_key(vk_code) {
                    // User pressed a modifier key (Ctrl, Alt, Shift, Win)
                    // Keep waiting for the base key
                    return CallNextHookEx(None, n_code, w_param, l_param);
                } else {
                    // Non-modifier key pressed! Combo is complete!
                    RECORDING_NON_MOD_PRESSED.store(true, Ordering::SeqCst);
                    RECORDING_MODE.store(0, Ordering::SeqCst);

                    let ctrl = current_ctrl_pressed();
                    let alt = current_alt_pressed();
                    let shift = current_shift_pressed();
                    let win = current_win_pressed();

                    let combo = KeyCombo::combo(ctrl, alt, shift, win, vk_code);
                    info!("Recorded hotkey combo: {:?}", combo);

                    if let Ok(guard) = HOOK_TX.lock() {
                        if let Some(tx) = guard.as_ref() {
                            let _ = tx.send(HotkeyEvent::KeyRecorded(rec_mode, combo));
                        }
                    }
                    return LRESULT(1); // Suppress base key
                }
            } else if is_up {
                // If a modifier was released and no non-modifier key was pressed,
                // the user intended to set just this single modifier key (e.g. Right Control)
                if is_modifier_key(vk_code) && !RECORDING_NON_MOD_PRESSED.load(Ordering::SeqCst) {
                    RECORDING_MODE.store(0, Ordering::SeqCst);
                    let combo = KeyCombo::single(vk_code);
                    info!("Recorded single modifier hotkey: {:?}", combo);

                    if let Ok(guard) = HOOK_TX.lock() {
                        if let Some(tx) = guard.as_ref() {
                            let _ = tx.send(HotkeyEvent::KeyRecorded(rec_mode, combo));
                        }
                    }
                    return LRESULT(1);
                }
            }
            return CallNextHookEx(None, n_code, w_param, l_param);
        }

        // 2. Normal hotkey monitoring
        let hotkey = KeyCombo::from_u32(HOTKEY_COMBO.load(Ordering::SeqCst));
        let toggle = KeyCombo::from_u32(TOGGLE_WINDOW_COMBO.load(Ordering::SeqCst));

        // Check Dictation Hotkey
        let is_hotkey_base = matches_vk(vk_code, hotkey.key);
        if is_hotkey_base || (HOTKEY_DOWN.load(Ordering::SeqCst) && is_up) {
            let ctrl_matches = hotkey.ctrl == current_ctrl_pressed()
                || (hotkey.ctrl && matches!(vk_code, 0x11 | 0xA2 | 0xA3));
            let alt_matches = hotkey.alt == current_alt_pressed()
                || (hotkey.alt && matches!(vk_code, 0x12 | 0xA4 | 0xA5));
            let shift_matches = hotkey.shift == current_shift_pressed()
                || (hotkey.shift && matches!(vk_code, 0x10 | 0xA0 | 0xA1));
            let win_matches = hotkey.win == current_win_pressed()
                || (hotkey.win && matches!(vk_code, 0x5B | 0x5C));

            let all_modifiers_match = ctrl_matches && alt_matches && shift_matches && win_matches;

            if is_down && is_hotkey_base && all_modifiers_match {
                if !HOTKEY_DOWN.load(Ordering::SeqCst) {
                    HOTKEY_DOWN.store(true, Ordering::SeqCst);
                    if let Ok(guard) = HOOK_TX.lock() {
                        if let Some(tx) = guard.as_ref() {
                            let _ = tx.send(HotkeyEvent::Pressed);
                        }
                    }
                }
                if hotkey.has_modifiers() {
                    return LRESULT(1); // Suppress key typing into application
                }
            } else if is_up && HOTKEY_DOWN.load(Ordering::SeqCst) {
                let is_released = is_hotkey_base
                    || (hotkey.ctrl && matches!(vk_code, 0x11 | 0xA2 | 0xA3))
                    || (hotkey.alt && matches!(vk_code, 0x12 | 0xA4 | 0xA5))
                    || (hotkey.shift && matches!(vk_code, 0x10 | 0xA0 | 0xA1))
                    || (hotkey.win && matches!(vk_code, 0x5B | 0x5C));

                if is_released {
                    HOTKEY_DOWN.store(false, Ordering::SeqCst);
                    if let Ok(guard) = HOOK_TX.lock() {
                        if let Some(tx) = guard.as_ref() {
                            let _ = tx.send(HotkeyEvent::Released);
                        }
                    }
                    if hotkey.has_modifiers() {
                        return LRESULT(1);
                    }
                }
            }
        }

        // Check Window Toggle Hotkey
        let is_toggle_base = matches_vk(vk_code, toggle.key);
        if is_toggle_base || (TOGGLE_DOWN.load(Ordering::SeqCst) && is_up) {
            let ctrl_matches = toggle.ctrl == current_ctrl_pressed()
                || (toggle.ctrl && matches!(vk_code, 0x11 | 0xA2 | 0xA3));
            let alt_matches = toggle.alt == current_alt_pressed()
                || (toggle.alt && matches!(vk_code, 0x12 | 0xA4 | 0xA5));
            let shift_matches = toggle.shift == current_shift_pressed()
                || (toggle.shift && matches!(vk_code, 0x10 | 0xA0 | 0xA1));
            let win_matches = toggle.win == current_win_pressed()
                || (toggle.win && matches!(vk_code, 0x5B | 0x5C));

            let all_modifiers_match = ctrl_matches && alt_matches && shift_matches && win_matches;

            if is_down && is_toggle_base && all_modifiers_match {
                if !TOGGLE_DOWN.load(Ordering::SeqCst) {
                    TOGGLE_DOWN.store(true, Ordering::SeqCst);
                    PlatformManager::toggle_main_window();
                }
                if toggle.has_modifiers() {
                    return LRESULT(1); // Suppress key
                }
            } else if is_up && TOGGLE_DOWN.load(Ordering::SeqCst) {
                let is_released = is_toggle_base
                    || (toggle.ctrl && matches!(vk_code, 0x11 | 0xA2 | 0xA3))
                    || (toggle.alt && matches!(vk_code, 0x12 | 0xA4 | 0xA5))
                    || (toggle.shift && matches!(vk_code, 0x10 | 0xA0 | 0xA1))
                    || (toggle.win && matches!(vk_code, 0x5B | 0x5C));

                if is_released {
                    TOGGLE_DOWN.store(false, Ordering::SeqCst);
                    if toggle.has_modifiers() {
                        return LRESULT(1);
                    }
                }
            }
        }
    }

    CallNextHookEx(None, n_code, w_param, l_param)
}

unsafe extern "system" fn hud_wnd_proc(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1), // Prevent flicker
        WM_TIMER => {
            HUD_TICK.fetch_add(1, Ordering::SeqCst);
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut rect);
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;

            let mem_dc = CreateCompatibleDC(hdc);
            let mem_bmp = CreateCompatibleBitmap(hdc, width, height);
            let old_bmp = SelectObject(mem_dc, mem_bmp);

            // 1. Draw rounded dark glass capsule
            let card_brush = CreateSolidBrush(COLORREF(0x001B120B)); // Deep obsidian slate
            let card_pen = CreatePen(PS_SOLID, 1, COLORREF(0x004A3622)); // Subtle glass border
            let old_brush = SelectObject(mem_dc, card_brush);
            let old_pen = SelectObject(mem_dc, card_pen);

            let _ = RoundRect(mem_dc, 0, 0, width, height, 28, 28);

            let _ = SelectObject(mem_dc, old_brush);
            let _ = SelectObject(mem_dc, old_pen);
            let _ = DeleteObject(card_brush);
            let _ = DeleteObject(card_pen);

            let state = HUD_STATE.load(Ordering::SeqCst);
            let tick = HUD_TICK.load(Ordering::SeqCst);
            let amp = f32::from_bits(HUD_AMPLITUDE.load(Ordering::SeqCst));

            let _ = SetBkMode(mem_dc, TRANSPARENT);

            // 2. Left indicator with pulsing aura & status text
            let (core_color, aura_color, label_text) = match state {
                1 => (
                    COLORREF(0x003344FF), // Bright coral red
                    COLORREF(0x001E1E66), // Soft red aura
                    "Listening...",
                ),
                2 => (
                    COLORREF(0x00F755A8), // Bright lavender violet
                    COLORREF(0x00662244), // Soft violet aura
                    "Transcribing...",
                ),
                3 => (
                    COLORREF(0x0033DD33), // Emerald green
                    COLORREF(0x001E551E), // Soft green aura
                    "Inserted!",
                ),
                _ => (COLORREF(0x00888888), COLORREF(0x00333333), "Ready"),
            };

            let null_pen = CreatePen(PS_NULL, 0, COLORREF(0));
            let old_pen_prev = SelectObject(mem_dc, null_pen);

            // Draw outer aura ring when active
            if state == 1 || state == 2 {
                let pulse = (tick as f32 * 0.22).sin().abs();
                let aura_r = 7 + (pulse * 3.5) as i32;
                let aura_brush = CreateSolidBrush(aura_color);
                let old_b = SelectObject(mem_dc, aura_brush);
                let _ = RoundRect(
                    mem_dc,
                    19 - aura_r,
                    29 - aura_r,
                    19 + aura_r,
                    29 + aura_r,
                    aura_r * 2,
                    aura_r * 2,
                );
                let _ = SelectObject(mem_dc, old_b);
                let _ = DeleteObject(aura_brush);
            }

            // Draw inner core circle
            let core_r = 4;
            let core_brush = CreateSolidBrush(core_color);
            let old_b2 = SelectObject(mem_dc, core_brush);
            let _ = RoundRect(
                mem_dc,
                19 - core_r,
                29 - core_r,
                19 + core_r,
                29 + core_r,
                core_r * 2,
                core_r * 2,
            );
            let _ = SelectObject(mem_dc, old_b2);
            let _ = DeleteObject(core_brush);
            let _ = SelectObject(mem_dc, old_pen_prev);
            let _ = DeleteObject(null_pen);

            // Status Text (Segoe UI Bold 15px)
            let font = CreateFontW(
                15,
                0,
                0,
                0,
                FW_BOLD.0 as i32,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                0,
                windows::core::w!("Segoe UI"),
            );
            let old_font = SelectObject(mem_dc, font);
            let _ = SetTextColor(mem_dc, COLORREF(0x00FFFFFF));

            let wide_label: Vec<u16> = label_text.encode_utf16().collect();
            let _ = TextOutW(mem_dc, 34, 11, &wide_label);

            let _ = SelectObject(mem_dc, old_font);
            let _ = DeleteObject(font);

            // Active Provider Badge
            let badge_str = HUD_BADGE.lock().unwrap_or_else(|e| e.into_inner()).clone();
            if !badge_str.is_empty() {
                let badge_font = CreateFontW(
                    11,
                    0,
                    0,
                    0,
                    FW_NORMAL.0 as i32,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    windows::core::w!("Segoe UI"),
                );
                let old_font2 = SelectObject(mem_dc, badge_font);
                let _ = SetTextColor(mem_dc, COLORREF(0x00F8C006)); // Electric Cyan
                let wide_badge: Vec<u16> = format!("⚡ {}", badge_str).encode_utf16().collect();
                let _ = TextOutW(mem_dc, 34, 33, &wide_badge);
                let _ = SelectObject(mem_dc, old_font2);
                let _ = DeleteObject(badge_font);
            }

            // 3. Right Animated Waveform (9 balanced spectrum bars)
            if state == 1 || state == 2 {
                let bar_count = 9;
                let start_x = 164;
                let bar_w = 4;
                let gap = 6;
                let center_y = 29;

                let bar_color = if state == 1 {
                    COLORREF(0x00F8C006) // Electric Cyan
                } else {
                    COLORREF(0x00F755A8) // Transcribing Violet
                };

                let bar_brush = CreateSolidBrush(bar_color);
                let null_pen2 = CreatePen(PS_NULL, 0, COLORREF(0));
                let old_b3 = SelectObject(mem_dc, bar_brush);
                let old_p3 = SelectObject(mem_dc, null_pen2);

                for i in 0..bar_count {
                    let bx = start_x + (i * (bar_w + gap));
                    let phase = (i as f32 * 0.40) + (tick as f32 * 0.35);
                    let bell = 1.0 - ((i as f32 - 4.0).abs() / 4.5) * 0.35;

                    let wave = if state == 1 {
                        let sin_val = phase.sin() * 0.5 + 0.5;
                        let height_boost = (amp * 48.0).clamp(2.0, 20.0);
                        (sin_val * height_boost * bell).max(2.5)
                    } else {
                        (phase.sin() * 0.5 + 0.5) * 8.0 * bell + 2.0
                    };

                    let half_h = wave as i32;
                    let _ = RoundRect(
                        mem_dc,
                        bx,
                        center_y - half_h,
                        bx + bar_w,
                        center_y + half_h + 1,
                        2,
                        2,
                    );
                }

                let _ = SelectObject(mem_dc, old_b3);
                let _ = SelectObject(mem_dc, old_p3);
                let _ = DeleteObject(bar_brush);
                let _ = DeleteObject(null_pen2);
            }

            // Copy back buffer to screen
            let _ = BitBlt(hdc, 0, 0, width, height, mem_dc, 0, 0, SRCCOPY);

            let _ = SelectObject(mem_dc, old_bmp);
            let _ = DeleteObject(mem_bmp);
            let _ = DeleteDC(mem_dc);

            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, w_param, l_param),
    }
}

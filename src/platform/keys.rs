//! Canonical virtual-key naming, shared by every platform backend.
//!
//! The Windows implementation could fall back to `GetKeyNameTextW` for
//! unknown keys; this portable version covers the keys a user can bind and
//! falls back to a hex representation.

/// Human readable name for a Windows virtual-key code.
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
        0xA3 => "Right Control".to_string(),
        0xA2 => "Left Control".to_string(),
        0x11 => "Control".to_string(),
        0xA5 => "Right Alt".to_string(),
        0xA4 => "Left Alt".to_string(),
        0x12 => "Alt".to_string(),
        0xA1 => "Right Shift".to_string(),
        0xA0 => "Left Shift".to_string(),
        0x10 => "Shift".to_string(),
        0x5B => "Left Win".to_string(),
        0x5C => "Right Win".to_string(),
        0x14 => "Caps Lock".to_string(),
        0x20 => "Space".to_string(),
        0x09 => "Tab".to_string(),
        0x0D => "Enter".to_string(),
        0x1B => "Escape".to_string(),
        0x08 => "Backspace".to_string(),
        0x2D => "Insert".to_string(),
        0x2E => "Delete".to_string(),
        0x24 => "Home".to_string(),
        0x23 => "End".to_string(),
        0x21 => "Page Up".to_string(),
        0x22 => "Page Down".to_string(),
        0x2C => "Print Screen".to_string(),
        0x91 => "Scroll Lock".to_string(),
        0x13 => "Pause / Break".to_string(),
        0xC0 => "Tilde (~)".to_string(),
        0x60..=0x69 => format!("NumPad {}", vk - 0x60),
        0x6A => "NumPad *".to_string(),
        0x6B => "NumPad +".to_string(),
        0x6D => "NumPad -".to_string(),
        0x6E => "NumPad .".to_string(),
        0x6F => "NumPad /".to_string(),
        0x30..=0x39 => format!("{}", (vk as u8) as char),
        0x41..=0x5A => format!("{}", (vk as u8) as char),
        _ => format!("Key (0x{:02X})", vk),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_keys() {
        assert_eq!(vk_to_string(0x77), "F8");
        assert_eq!(vk_to_string(0x78), "F9");
        assert_eq!(vk_to_string(0x20), "Space");
        assert_eq!(vk_to_string(0x41), "A");
        assert_eq!(vk_to_string(0x30), "0");
        assert_eq!(vk_to_string(0x11), "Control");
        assert_eq!(vk_to_string(0x5B), "Left Win");
    }

    #[test]
    fn unknown_keys_fall_back_to_hex() {
        assert_eq!(vk_to_string(0x01), "Key (0x01)");
    }
}

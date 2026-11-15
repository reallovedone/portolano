//! `KeyboardEvent.code` -> PS/2 set-1 scancode (extended keys carry the `0xE000` flag).
//!
//! Identical to `ui/src/lib/sessions/keymap.ts`; a test below parses that file and fails when
//! the two tables differ.

use ironrdp_input::Scancode;

const E: u16 = 0xE000;

pub(crate) const TABLE: &[(&str, u16)] = &[
    ("Escape", 0x01),
    ("Digit1", 0x02),
    ("Digit2", 0x03),
    ("Digit3", 0x04),
    ("Digit4", 0x05),
    ("Digit5", 0x06),
    ("Digit6", 0x07),
    ("Digit7", 0x08),
    ("Digit8", 0x09),
    ("Digit9", 0x0a),
    ("Digit0", 0x0b),
    ("Minus", 0x0c),
    ("Equal", 0x0d),
    ("Backspace", 0x0e),
    ("Tab", 0x0f),
    ("KeyQ", 0x10),
    ("KeyW", 0x11),
    ("KeyE", 0x12),
    ("KeyR", 0x13),
    ("KeyT", 0x14),
    ("KeyY", 0x15),
    ("KeyU", 0x16),
    ("KeyI", 0x17),
    ("KeyO", 0x18),
    ("KeyP", 0x19),
    ("BracketLeft", 0x1a),
    ("BracketRight", 0x1b),
    ("Enter", 0x1c),
    ("ControlLeft", 0x1d),
    ("KeyA", 0x1e),
    ("KeyS", 0x1f),
    ("KeyD", 0x20),
    ("KeyF", 0x21),
    ("KeyG", 0x22),
    ("KeyH", 0x23),
    ("KeyJ", 0x24),
    ("KeyK", 0x25),
    ("KeyL", 0x26),
    ("Semicolon", 0x27),
    ("Quote", 0x28),
    ("Backquote", 0x29),
    ("ShiftLeft", 0x2a),
    ("Backslash", 0x2b),
    ("KeyZ", 0x2c),
    ("KeyX", 0x2d),
    ("KeyC", 0x2e),
    ("KeyV", 0x2f),
    ("KeyB", 0x30),
    ("KeyN", 0x31),
    ("KeyM", 0x32),
    ("Comma", 0x33),
    ("Period", 0x34),
    ("Slash", 0x35),
    ("ShiftRight", 0x36),
    ("NumpadMultiply", 0x37),
    ("AltLeft", 0x38),
    ("Space", 0x39),
    ("CapsLock", 0x3a),
    ("F1", 0x3b),
    ("F2", 0x3c),
    ("F3", 0x3d),
    ("F4", 0x3e),
    ("F5", 0x3f),
    ("F6", 0x40),
    ("F7", 0x41),
    ("F8", 0x42),
    ("F9", 0x43),
    ("F10", 0x44),
    ("NumLock", 0x45),
    ("ScrollLock", 0x46),
    ("Numpad7", 0x47),
    ("Numpad8", 0x48),
    ("Numpad9", 0x49),
    ("NumpadSubtract", 0x4a),
    ("Numpad4", 0x4b),
    ("Numpad5", 0x4c),
    ("Numpad6", 0x4d),
    ("NumpadAdd", 0x4e),
    ("Numpad1", 0x4f),
    ("Numpad2", 0x50),
    ("Numpad3", 0x51),
    ("Numpad0", 0x52),
    ("NumpadDecimal", 0x53),
    ("IntlBackslash", 0x56),
    ("F11", 0x57),
    ("F12", 0x58),
    ("NumpadEqual", 0x59),
    ("F13", 0x64),
    ("F14", 0x65),
    ("F15", 0x66),
    ("F16", 0x67),
    ("F17", 0x68),
    ("F18", 0x69),
    ("F19", 0x6a),
    ("F20", 0x6b),
    ("F21", 0x6c),
    ("F22", 0x6d),
    ("F23", 0x6e),
    ("F24", 0x76),
    ("KanaMode", 0x70),
    ("IntlRo", 0x73),
    ("Convert", 0x79),
    ("NonConvert", 0x7b),
    ("IntlYen", 0x7d),
    ("NumpadComma", 0x7e),
    ("NumpadEnter", E | 0x1c),
    ("ControlRight", E | 0x1d),
    ("NumpadDivide", E | 0x35),
    ("PrintScreen", E | 0x37),
    ("AltRight", E | 0x38),
    ("Home", E | 0x47),
    ("ArrowUp", E | 0x48),
    ("PageUp", E | 0x49),
    ("ArrowLeft", E | 0x4b),
    ("ArrowRight", E | 0x4d),
    ("End", E | 0x4f),
    ("ArrowDown", E | 0x50),
    ("PageDown", E | 0x51),
    ("Insert", E | 0x52),
    ("Delete", E | 0x53),
    ("MetaLeft", E | 0x5b),
    ("MetaRight", E | 0x5c),
    ("ContextMenu", E | 0x5d),
];

/// The scancode of a DOM `KeyboardEvent.code`, `None` for keys RDP has no equivalent for.
pub fn scancode_for(code: &str) -> Option<Scancode> {
    TABLE
        .iter()
        .find(|(name, _)| *name == code)
        .map(|(_, value)| Scancode::from_u16(*value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_keys_map_to_the_set1_codes() {
        assert_eq!(scancode_for("KeyA"), Some(Scancode::from_u8(false, 0x1e)));
        assert_eq!(scancode_for("Enter"), Some(Scancode::from_u8(false, 0x1c)));
        assert_eq!(
            scancode_for("NumpadEnter"),
            Some(Scancode::from_u8(true, 0x1c))
        );
        assert_eq!(
            scancode_for("ArrowLeft"),
            Some(Scancode::from_u8(true, 0x4b))
        );
        assert_eq!(
            scancode_for("ControlRight"),
            Some(Scancode::from_u8(true, 0x1d))
        );
        assert_eq!(
            scancode_for("IntlBackslash"),
            Some(Scancode::from_u8(false, 0x56))
        );
        assert_eq!(scancode_for("Unidentified"), None);
    }

    #[test]
    fn codes_are_unique() {
        let mut names: Vec<&str> = TABLE.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), TABLE.len());
    }

    /// The UI table and this one must be the same.
    #[test]
    fn matches_the_ui_table() {
        let ts = include_str!("../../../ui/src/lib/sessions/keymap.ts");
        let mut ui: Vec<(String, u16)> = Vec::new();
        for line in ts.lines() {
            let line = line.trim();
            let Some((name, rest)) = line.split_once(": ") else {
                continue;
            };
            let Some(value) = rest.strip_suffix(',') else {
                continue;
            };
            if !name.chars().all(|c| c.is_ascii_alphanumeric()) {
                continue;
            }
            let (ext, hex) = match value.strip_prefix("E | ") {
                Some(h) => (true, h),
                None => (false, value),
            };
            let Some(hex) = hex.strip_prefix("0x") else {
                continue;
            };
            let Ok(code) = u16::from_str_radix(hex, 16) else {
                continue;
            };
            ui.push((name.to_owned(), if ext { E | code } else { code }));
        }
        let rust: Vec<(String, u16)> = TABLE.iter().map(|(n, v)| ((*n).to_owned(), *v)).collect();
        assert_eq!(ui, rust);
    }
}

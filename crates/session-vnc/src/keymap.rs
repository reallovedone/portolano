//! `KeyboardEvent.code` -> X11 keysyms for RFB key events.
//!
//! The UI sends the physical key (`code`), never the character it would produce, so the
//! keysym is chosen for a **US layout**: a physical `KeyA` is `a`, or `A` while Shift is held,
//! `Digit1` is `1` or `!`, and so on. On a remote with another layout the keysym is still
//! resolved by the server to whatever key produces that character, so letters and digits
//! mostly come out right, while punctuation typed on a non-US keyboard does not (the layout
//! of the local machine is not known here). Caps Lock is sent as a key but not tracked: the
//! case of letters follows Shift only.
//!
//! The set of codes is exactly the one in `ui/src/lib/sessions/keymap.ts`; a test parses that
//! file and fails when the two drift apart.

/// `(code, keysym, keysym while Shift is held)`.
pub(crate) type Entry = (&'static str, u32, u32);

const fn k(code: &'static str, keysym: u32) -> Entry {
    (code, keysym, keysym)
}

const fn kk(code: &'static str, base: u32, shifted: u32) -> Entry {
    (code, base, shifted)
}

pub(crate) const TABLE: &[Entry] = &[
    k("Escape", 0xff1b),
    kk("Digit1", 0x0031, 0x0021),
    kk("Digit2", 0x0032, 0x0040),
    kk("Digit3", 0x0033, 0x0023),
    kk("Digit4", 0x0034, 0x0024),
    kk("Digit5", 0x0035, 0x0025),
    kk("Digit6", 0x0036, 0x005e),
    kk("Digit7", 0x0037, 0x0026),
    kk("Digit8", 0x0038, 0x002a),
    kk("Digit9", 0x0039, 0x0028),
    kk("Digit0", 0x0030, 0x0029),
    kk("Minus", 0x002d, 0x005f),
    kk("Equal", 0x003d, 0x002b),
    k("Backspace", 0xff08),
    k("Tab", 0xff09),
    kk("KeyQ", 0x0071, 0x0051),
    kk("KeyW", 0x0077, 0x0057),
    kk("KeyE", 0x0065, 0x0045),
    kk("KeyR", 0x0072, 0x0052),
    kk("KeyT", 0x0074, 0x0054),
    kk("KeyY", 0x0079, 0x0059),
    kk("KeyU", 0x0075, 0x0055),
    kk("KeyI", 0x0069, 0x0049),
    kk("KeyO", 0x006f, 0x004f),
    kk("KeyP", 0x0070, 0x0050),
    kk("BracketLeft", 0x005b, 0x007b),
    kk("BracketRight", 0x005d, 0x007d),
    k("Enter", 0xff0d),
    k("ControlLeft", 0xffe3),
    kk("KeyA", 0x0061, 0x0041),
    kk("KeyS", 0x0073, 0x0053),
    kk("KeyD", 0x0064, 0x0044),
    kk("KeyF", 0x0066, 0x0046),
    kk("KeyG", 0x0067, 0x0047),
    kk("KeyH", 0x0068, 0x0048),
    kk("KeyJ", 0x006a, 0x004a),
    kk("KeyK", 0x006b, 0x004b),
    kk("KeyL", 0x006c, 0x004c),
    kk("Semicolon", 0x003b, 0x003a),
    kk("Quote", 0x0027, 0x0022),
    kk("Backquote", 0x0060, 0x007e),
    k("ShiftLeft", 0xffe1),
    kk("Backslash", 0x005c, 0x007c),
    kk("KeyZ", 0x007a, 0x005a),
    kk("KeyX", 0x0078, 0x0058),
    kk("KeyC", 0x0063, 0x0043),
    kk("KeyV", 0x0076, 0x0056),
    kk("KeyB", 0x0062, 0x0042),
    kk("KeyN", 0x006e, 0x004e),
    kk("KeyM", 0x006d, 0x004d),
    kk("Comma", 0x002c, 0x003c),
    kk("Period", 0x002e, 0x003e),
    kk("Slash", 0x002f, 0x003f),
    k("ShiftRight", 0xffe2),
    k("NumpadMultiply", 0xffaa),
    k("AltLeft", 0xffe9),
    k("Space", 0x0020),
    k("CapsLock", 0xffe5),
    k("F1", 0xffbe),
    k("F2", 0xffbf),
    k("F3", 0xffc0),
    k("F4", 0xffc1),
    k("F5", 0xffc2),
    k("F6", 0xffc3),
    k("F7", 0xffc4),
    k("F8", 0xffc5),
    k("F9", 0xffc6),
    k("F10", 0xffc7),
    k("NumLock", 0xff7f),
    k("ScrollLock", 0xff14),
    k("Numpad7", 0xffb7),
    k("Numpad8", 0xffb8),
    k("Numpad9", 0xffb9),
    k("NumpadSubtract", 0xffad),
    k("Numpad4", 0xffb4),
    k("Numpad5", 0xffb5),
    k("Numpad6", 0xffb6),
    k("NumpadAdd", 0xffab),
    k("Numpad1", 0xffb1),
    k("Numpad2", 0xffb2),
    k("Numpad3", 0xffb3),
    k("Numpad0", 0xffb0),
    k("NumpadDecimal", 0xffae),
    kk("IntlBackslash", 0x003c, 0x003e),
    k("F11", 0xffc8),
    k("F12", 0xffc9),
    k("NumpadEqual", 0xffbd),
    k("F13", 0xffca),
    k("F14", 0xffcb),
    k("F15", 0xffcc),
    k("F16", 0xffcd),
    k("F17", 0xffce),
    k("F18", 0xffcf),
    k("F19", 0xffd0),
    k("F20", 0xffd1),
    k("F21", 0xffd2),
    k("F22", 0xffd3),
    k("F23", 0xffd4),
    k("F24", 0xffd5),
    k("KanaMode", 0xff27),
    kk("IntlRo", 0x005c, 0x005f),
    k("Convert", 0xff23),
    k("NonConvert", 0xff22),
    k("IntlYen", 0x00a5),
    k("NumpadComma", 0xffac),
    k("NumpadEnter", 0xff8d),
    k("ControlRight", 0xffe4),
    k("NumpadDivide", 0xffaf),
    k("PrintScreen", 0xff61),
    k("AltRight", 0xffea),
    k("Home", 0xff50),
    k("ArrowUp", 0xff52),
    k("PageUp", 0xff55),
    k("ArrowLeft", 0xff51),
    k("ArrowRight", 0xff53),
    k("End", 0xff57),
    k("ArrowDown", 0xff54),
    k("PageDown", 0xff56),
    k("Insert", 0xff63),
    k("Delete", 0xffff),
    k("MetaLeft", 0xffeb),
    k("MetaRight", 0xffec),
    k("ContextMenu", 0xff67),
];

/// The table entry of a DOM `KeyboardEvent.code`, `None` for keys VNC has no equivalent for.
pub(crate) fn lookup(code: &str) -> Option<&'static Entry> {
    TABLE.iter().find(|(name, _, _)| *name == code)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keysym(code: &str, shift: bool) -> Option<u32> {
        lookup(code).map(|(_, base, shifted)| if shift { *shifted } else { *base })
    }

    #[test]
    fn common_keys_follow_the_us_layout() {
        assert_eq!(keysym("KeyA", false), Some(0x61));
        assert_eq!(keysym("KeyA", true), Some(0x41));
        assert_eq!(keysym("Digit1", true), Some(0x21));
        assert_eq!(keysym("Digit0", true), Some(0x29));
        assert_eq!(keysym("Slash", true), Some(u32::from(b'?')));
        assert_eq!(keysym("Enter", false), Some(0xff0d));
        assert_eq!(keysym("Escape", false), Some(0xff1b));
        assert_eq!(keysym("Space", false), Some(0x20));
        assert_eq!(keysym("ArrowLeft", false), Some(0xff51));
        assert_eq!(keysym("Delete", false), Some(0xffff));
        assert_eq!(keysym("F1", false), Some(0xffbe));
        assert_eq!(keysym("F12", false), Some(0xffc9));
        assert_eq!(keysym("F24", false), Some(0xffd5));
        assert_eq!(keysym("Numpad0", false), Some(0xffb0));
        assert_eq!(keysym("NumpadEnter", false), Some(0xff8d));
        assert_eq!(keysym("ShiftLeft", false), Some(0xffe1));
        assert_eq!(keysym("MetaLeft", false), Some(0xffeb));
        assert_eq!(keysym("Unidentified", false), None);
    }

    #[test]
    fn codes_are_unique() {
        let mut names: Vec<&str> = TABLE.iter().map(|(n, _, _)| *n).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), TABLE.len());
    }

    /// Every code the UI can send has a keysym, and no keysym is defined for a code the UI
    /// never sends.
    #[test]
    fn matches_the_ui_table() {
        let ts = include_str!("../../../ui/src/lib/sessions/keymap.ts");
        let mut ui: Vec<String> = Vec::new();
        for line in ts.lines() {
            let line = line.trim();
            let Some((name, rest)) = line.split_once(": ") else {
                continue;
            };
            let Some(value) = rest.strip_suffix(',') else {
                continue;
            };
            let hex = value.strip_prefix("E | ").unwrap_or(value);
            if name.chars().all(|c| c.is_ascii_alphanumeric())
                && hex
                    .strip_prefix("0x")
                    .is_some_and(|h| u16::from_str_radix(h, 16).is_ok())
            {
                ui.push(name.to_owned());
            }
        }
        let mut rust: Vec<String> = TABLE.iter().map(|(n, _, _)| (*n).to_owned()).collect();
        ui.sort_unstable();
        rust.sort_unstable();
        assert!(ui.len() > 100, "the UI table was not parsed");
        assert_eq!(ui, rust);
    }
}

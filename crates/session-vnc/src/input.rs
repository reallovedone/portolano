//! Translation of UI input into RFB key and pointer events.
//!
//! The UI sends physical keys (`KeyboardEvent.code`) and the DOM button mask; this module
//! keeps just enough state to produce a consistent event stream: which keys are down (with
//! the keysym each was pressed as, so the release matches even when Shift changed meanwhile),
//! which buttons are down, the last pointer position and the remainder of a smooth wheel.

use crate::keymap;
use crate::rfb::{key_event, pointer_event};

/// RFB button bits (bit 0 left, bit 1 middle, bit 2 right, bits 3 and 4 the wheel).
const RFB_LEFT: u8 = 1;
const RFB_MIDDLE: u8 = 2;
const RFB_RIGHT: u8 = 4;
const RFB_WHEEL_UP: u8 = 8;
const RFB_WHEEL_DOWN: u8 = 16;

/// One wheel notch in the units the UI sends (the same as RDP).
const WHEEL_NOTCH: i32 = 120;
/// Bound on the clicks one command may produce.
const MAX_WHEEL_CLICKS: i32 = 32;

/// DOM `MouseEvent.buttons` (1 left, 2 right, 4 middle) to the RFB mask. The back and
/// forward buttons have no RFB equivalent and are dropped.
pub fn rfb_buttons(dom: u8) -> u8 {
    let mut mask = 0;
    if dom & 1 != 0 {
        mask |= RFB_LEFT;
    }
    if dom & 2 != 0 {
        mask |= RFB_RIGHT;
    }
    if dom & 4 != 0 {
        mask |= RFB_MIDDLE;
    }
    mask
}

#[derive(Debug, Default)]
pub struct InputState {
    /// Keys that are down, with the keysym they were pressed as.
    pressed: Vec<(&'static str, u32)>,
    buttons: u8,
    x: u16,
    y: u16,
    wheel_rest: i32,
}

impl InputState {
    fn shift_down(&self) -> bool {
        self.pressed
            .iter()
            .any(|(code, _)| *code == "ShiftLeft" || *code == "ShiftRight")
    }

    /// Appends the RFB message for a key press or release. Unknown keys and releases of keys
    /// that are not down produce nothing.
    pub fn key(&mut self, code: &str, down: bool, out: &mut Vec<u8>) {
        let Some((name, base, shifted)) = keymap::lookup(code) else {
            return;
        };
        if down {
            let keysym = if self.shift_down() { *shifted } else { *base };
            // A repeat of a key that is already down keeps its keysym.
            let keysym = match self.pressed.iter_mut().find(|(c, _)| c == name) {
                Some((_, held)) => *held,
                None => {
                    self.pressed.push((name, keysym));
                    keysym
                }
            };
            out.extend_from_slice(&key_event(true, keysym));
        } else if let Some(index) = self.pressed.iter().position(|(c, _)| c == name) {
            let (_, keysym) = self.pressed.swap_remove(index);
            out.extend_from_slice(&key_event(false, keysym));
        }
    }

    /// Appends the pointer events for a move/button change and the wheel clicks. `width` and
    /// `height` are the size of the remote desktop (the position is clamped to it).
    pub fn pointer(
        &mut self,
        x: u16,
        y: u16,
        dom_buttons: u8,
        wheel: i16,
        (width, height): (u16, u16),
        out: &mut Vec<u8>,
    ) {
        self.x = x.min(width.saturating_sub(1));
        self.y = y.min(height.saturating_sub(1));
        self.buttons = rfb_buttons(dom_buttons);
        out.extend_from_slice(&pointer_event(self.buttons, self.x, self.y));
        if wheel == 0 {
            return;
        }
        // Positive is down. A smooth wheel (trackpad) sends fractions of a notch: they add up
        // and click when a whole notch is reached; a change of direction starts over.
        let delta = i32::from(wheel);
        if (self.wheel_rest > 0) != (delta > 0) {
            self.wheel_rest = 0;
        }
        self.wheel_rest += delta;
        let clicks = (self.wheel_rest / WHEEL_NOTCH).clamp(-MAX_WHEEL_CLICKS, MAX_WHEEL_CLICKS);
        self.wheel_rest -= clicks * WHEEL_NOTCH;
        let button = if clicks > 0 {
            RFB_WHEEL_DOWN
        } else {
            RFB_WHEEL_UP
        };
        for _ in 0..clicks.abs() {
            out.extend_from_slice(&pointer_event(self.buttons | button, self.x, self.y));
            out.extend_from_slice(&pointer_event(self.buttons, self.x, self.y));
        }
    }

    /// Releases every key and button that is down (hidden tab, closing).
    pub fn release_all(&mut self, out: &mut Vec<u8>) {
        for (_, keysym) in self.pressed.drain(..) {
            out.extend_from_slice(&key_event(false, keysym));
        }
        if self.buttons != 0 {
            self.buttons = 0;
            out.extend_from_slice(&pointer_event(0, self.x, self.y));
        }
        self.wheel_rest = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: (u16, u16) = (100, 50);

    fn messages(out: &[u8], len: usize) -> Vec<&[u8]> {
        out.chunks(len).collect()
    }

    #[test]
    fn dom_buttons_map_to_rfb_bits() {
        assert_eq!(rfb_buttons(0), 0);
        assert_eq!(rfb_buttons(1), 1);
        assert_eq!(rfb_buttons(2), 4);
        assert_eq!(rfb_buttons(4), 2);
        assert_eq!(rfb_buttons(1 | 2 | 4), 7);
        // Back and forward are dropped.
        assert_eq!(rfb_buttons(8 | 16), 0);
        assert_eq!(rfb_buttons(8 | 1), 1);
    }

    #[test]
    fn pointer_events_carry_the_mask_and_a_clamped_position() {
        let mut input = InputState::default();
        let mut out = Vec::new();
        input.pointer(10, 20, 2, 0, SIZE, &mut out);
        assert_eq!(out, pointer_event(4, 10, 20));
        out.clear();
        input.pointer(500, 500, 0, 0, SIZE, &mut out);
        assert_eq!(out, pointer_event(0, 99, 49));
    }

    #[test]
    fn wheel_notches_become_press_release_pairs_of_buttons_4_and_5() {
        let mut input = InputState::default();
        let mut out = Vec::new();
        // Two notches down with the left button held.
        input.pointer(5, 6, 1, 240, SIZE, &mut out);
        let m = messages(&out, 6);
        assert_eq!(m.len(), 1 + 4);
        assert_eq!(m[0], pointer_event(1, 5, 6));
        assert_eq!(m[1], pointer_event(1 | 16, 5, 6));
        assert_eq!(m[2], pointer_event(1, 5, 6));
        assert_eq!(m[3], pointer_event(1 | 16, 5, 6));
        assert_eq!(m[4], pointer_event(1, 5, 6));
        // One notch up.
        out.clear();
        input.pointer(5, 6, 0, -120, SIZE, &mut out);
        let m = messages(&out, 6);
        assert_eq!(
            m,
            vec![
                &pointer_event(0, 5, 6)[..],
                &pointer_event(8, 5, 6)[..],
                &pointer_event(0, 5, 6)[..]
            ]
        );
    }

    #[test]
    fn small_wheel_deltas_accumulate_and_a_reversal_starts_over() {
        let mut input = InputState::default();
        let clicks = |input: &mut InputState, wheel: i16| {
            let mut out = Vec::new();
            input.pointer(0, 0, 0, wheel, SIZE, &mut out);
            (out.len() / 6).saturating_sub(1) / 2
        };
        assert_eq!(clicks(&mut input, 50), 0);
        assert_eq!(clicks(&mut input, 50), 0);
        assert_eq!(clicks(&mut input, 50), 1);
        // 30 left over; reversing drops it.
        assert_eq!(clicks(&mut input, -100), 0);
        assert_eq!(clicks(&mut input, -20), 1);
    }

    #[test]
    fn keys_follow_shift_and_release_with_the_keysym_they_were_pressed_as() {
        let mut input = InputState::default();
        let mut out = Vec::new();
        input.key("KeyA", true, &mut out);
        input.key("KeyA", false, &mut out);
        input.key("ShiftLeft", true, &mut out);
        input.key("KeyA", true, &mut out);
        input.key("ShiftLeft", false, &mut out);
        // Shift is up now, but A was pressed as 'A' and is released as 'A'.
        input.key("KeyA", false, &mut out);
        let m = messages(&out, 8);
        assert_eq!(m[0], key_event(true, 0x61));
        assert_eq!(m[1], key_event(false, 0x61));
        assert_eq!(m[2], key_event(true, 0xffe1));
        assert_eq!(m[3], key_event(true, 0x41));
        assert_eq!(m[4], key_event(false, 0xffe1));
        assert_eq!(m[5], key_event(false, 0x41));
        assert_eq!(m.len(), 6);
    }

    #[test]
    fn unknown_keys_and_stray_releases_send_nothing() {
        let mut input = InputState::default();
        let mut out = Vec::new();
        input.key("Unidentified", true, &mut out);
        input.key("KeyQ", false, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn release_all_lifts_keys_and_buttons_once() {
        let mut input = InputState::default();
        let mut out = Vec::new();
        input.key("ControlLeft", true, &mut out);
        input.key("KeyC", true, &mut out);
        input.pointer(7, 8, 1 | 4, 0, SIZE, &mut out);
        out.clear();
        input.release_all(&mut out);
        let keys: Vec<&[u8]> = out[..16].chunks(8).collect();
        assert!(keys.contains(&&key_event(false, 0xffe3)[..]));
        assert!(keys.contains(&&key_event(false, u32::from(b'c'))[..]));
        assert_eq!(&out[16..], pointer_event(0, 7, 8));
        out.clear();
        input.release_all(&mut out);
        assert!(out.is_empty());
    }
}

//! `NSEvent` → `locus_ide_protocol::UiEvent` translation.
//!
//! The Windows IDE synthesises `UiEvent`s from Win32 messages
//! (`WM_KEYDOWN`/`WM_CHAR`/`WM_*BUTTON*`). On macOS the same events arrive as
//! `NSEvent`s delivered to the content `NSView`. To keep the protocol identical
//! across platforms, `UiEvent::Key.virtual_key` carries **Win32 virtual-key
//! codes** and `modifiers` uses the bit layout in [`modifiers`] — so the worker
//! side sees the same constants regardless of host OS.
//!
//! The mapping functions are intentionally **pure** — they take raw integers,
//! not `NSEvent` objects — so the whole translation table is unit-testable
//! headlessly. `window.rs` reads the values off the real `NSEvent` and calls
//! these. Adapted from MacNCL's `igui_mac::events`.

use locus_ide_protocol::event::{KeyState, MouseButton, MouseEvent, MouseOp, UiEvent};

/// The `UiEvent` modifier bit layout (host-neutral; both the macOS translation
/// here and the Windows IDE populate `modifiers` with these). Command on macOS
/// maps to `COMMAND` (the accelerator key, analogous to Ctrl on Windows menus).
pub mod modifiers {
    pub const SHIFT: u32 = 1 << 0;
    pub const CONTROL: u32 = 1 << 1;
    pub const ALT: u32 = 1 << 2; // macOS Option
    pub const COMMAND: u32 = 1 << 3; // macOS Command / Windows Super
    pub const CAPS: u32 = 1 << 4;
}

/// `NSEventModifierFlags` bit positions (from `<AppKit/NSEvent.h>`).
pub mod nsflags {
    pub const CAPS_LOCK: u64 = 1 << 16;
    pub const SHIFT: u64 = 1 << 17;
    pub const CONTROL: u64 = 1 << 18;
    pub const OPTION: u64 = 1 << 19;
    pub const COMMAND: u64 = 1 << 20;
}

/// macOS hardware virtual keycodes (`<Carbon/HIToolbox/Events.h>`, `kVK_*`).
pub mod kvk {
    pub const RETURN: u16 = 0x24;
    pub const TAB: u16 = 0x30;
    pub const SPACE: u16 = 0x31;
    pub const DELETE: u16 = 0x33; // Backspace
    pub const ESCAPE: u16 = 0x35;
    pub const FORWARD_DELETE: u16 = 0x75;
    pub const HOME: u16 = 0x73;
    pub const END: u16 = 0x77;
    pub const PAGE_UP: u16 = 0x74;
    pub const PAGE_DOWN: u16 = 0x79;
    pub const LEFT: u16 = 0x7B;
    pub const RIGHT: u16 = 0x7C;
    pub const DOWN: u16 = 0x7D;
    pub const UP: u16 = 0x7E;
    pub const F1: u16 = 0x7A;
    pub const F2: u16 = 0x78;
    pub const F3: u16 = 0x63;
    pub const F4: u16 = 0x76;
    pub const F5: u16 = 0x60;
    pub const F6: u16 = 0x61;
    pub const F7: u16 = 0x62;
    pub const F8: u16 = 0x64;
    pub const F9: u16 = 0x65;
    pub const F10: u16 = 0x6D;
    pub const F11: u16 = 0x67;
    pub const F12: u16 = 0x6F;
    pub const SLASH: u16 = 0x2C; // kVK_ANSI_Slash
}

/// Win32 virtual-key codes we map onto (the values the worker side expects).
pub mod vk {
    pub const BACK: u32 = 0x08;
    pub const TAB: u32 = 0x09;
    pub const RETURN: u32 = 0x0D;
    pub const ESCAPE: u32 = 0x1B;
    pub const SPACE: u32 = 0x20;
    pub const PRIOR: u32 = 0x21; // Page Up
    pub const NEXT: u32 = 0x22; // Page Down
    pub const END: u32 = 0x23;
    pub const HOME: u32 = 0x24;
    pub const LEFT: u32 = 0x25;
    pub const UP: u32 = 0x26;
    pub const RIGHT: u32 = 0x27;
    pub const DOWN: u32 = 0x28;
    pub const DELETE: u32 = 0x2E;
    pub const F1: u32 = 0x70;
    pub const OEM_2: u32 = 0xBF; // '/' '?'
}

/// Translate `NSEventModifierFlags` into the [`modifiers`] bitmask.
pub fn mods_from_flags(flags: u64) -> u32 {
    let mut m = 0;
    if flags & nsflags::SHIFT != 0 {
        m |= modifiers::SHIFT;
    }
    if flags & nsflags::CONTROL != 0 {
        m |= modifiers::CONTROL;
    }
    if flags & nsflags::OPTION != 0 {
        m |= modifiers::ALT;
    }
    if flags & nsflags::COMMAND != 0 {
        m |= modifiers::COMMAND;
    }
    if flags & nsflags::CAPS_LOCK != 0 {
        m |= modifiers::CAPS;
    }
    m
}

/// Map a macOS hardware keycode to a Win32 VK for the non-printable keys the
/// editor cares about. Returns `0` for keys best identified by their character
/// (letters/digits/punctuation) — [`vkey_from_char`] covers those.
pub fn vkey_from_keycode(keycode: u16) -> u32 {
    match keycode {
        kvk::RETURN => vk::RETURN,
        kvk::TAB => vk::TAB,
        kvk::SPACE => vk::SPACE,
        kvk::DELETE => vk::BACK,
        kvk::FORWARD_DELETE => vk::DELETE,
        kvk::ESCAPE => vk::ESCAPE,
        kvk::HOME => vk::HOME,
        kvk::END => vk::END,
        kvk::PAGE_UP => vk::PRIOR,
        kvk::PAGE_DOWN => vk::NEXT,
        kvk::LEFT => vk::LEFT,
        kvk::RIGHT => vk::RIGHT,
        kvk::UP => vk::UP,
        kvk::DOWN => vk::DOWN,
        kvk::F1 => vk::F1,
        kvk::F2 => vk::F1 + 1,
        kvk::F3 => vk::F1 + 2,
        kvk::F4 => vk::F1 + 3,
        kvk::F5 => vk::F1 + 4,
        kvk::F6 => vk::F1 + 5,
        kvk::F7 => vk::F1 + 6,
        kvk::F8 => vk::F1 + 7,
        kvk::F9 => vk::F1 + 8,
        kvk::F10 => vk::F1 + 9,
        kvk::F11 => vk::F1 + 10,
        kvk::F12 => vk::F1 + 11,
        kvk::SLASH => vk::OEM_2,
        _ => 0,
    }
}

/// Win32 VK for a printable character: letters → 'A'..'Z' (0x41..0x5A), digits
/// → '0'..'9' (0x30..0x39). Other characters return 0; the worker uses the
/// `Char` event's codepoint for those.
pub fn vkey_from_char(c: char) -> u32 {
    match c {
        'a'..='z' => (c as u32 - 'a' as u32) + 0x41,
        'A'..='Z' => (c as u32 - 'A' as u32) + 0x41,
        '0'..='9' => c as u32,
        _ => 0,
    }
}

/// Resolve a key event's `virtual_key`: prefer the hardware-keycode mapping (for
/// named keys), fall back to the character mapping (for letters/digits).
pub fn resolve_vkey(keycode: u16, chars_ignoring_mods: Option<char>) -> u32 {
    let vk = vkey_from_keycode(keycode);
    if vk != 0 {
        return vk;
    }
    chars_ignoring_mods.map(vkey_from_char).unwrap_or(0)
}

/// Convert a y coordinate from `NSEvent`'s window space (bottom-left origin,
/// y-up) to the top-left, y-down view space matching the `DrawCmd` IR.
/// `view_height` is the content view's height in points.
#[inline]
pub fn to_top_left_y(y_window: f64, view_height: f64) -> f64 {
    view_height - y_window
}

// ── UiEvent builders (window.rs calls these once it has the NSEvent) ────────

pub fn key_event(keycode: u16, chars_ignoring_mods: Option<char>, flags: u64, down: bool) -> UiEvent {
    UiEvent::Key {
        state: if down { KeyState::Down } else { KeyState::Up },
        virtual_key: resolve_vkey(keycode, chars_ignoring_mods),
        modifiers: mods_from_flags(flags),
    }
}

pub fn char_event(codepoint: u32, flags: u64) -> UiEvent {
    UiEvent::Char {
        codepoint,
        modifiers: mods_from_flags(flags),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn mouse_event(
    x: f64,
    y_top_left: f64,
    op: MouseOp,
    button: MouseButton,
    flags: u64,
    wheel_delta: i32,
    time_ms: u64,
) -> UiEvent {
    UiEvent::Mouse(MouseEvent {
        x: x as i32,
        y: y_top_left as i32,
        op,
        button,
        wheel_delta,
        modifiers: mods_from_flags(flags),
        time_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_flags_map_to_bits() {
        assert_eq!(mods_from_flags(0), 0);
        assert_eq!(mods_from_flags(nsflags::SHIFT), modifiers::SHIFT);
        assert_eq!(mods_from_flags(nsflags::CONTROL), modifiers::CONTROL);
        assert_eq!(mods_from_flags(nsflags::OPTION), modifiers::ALT);
        assert_eq!(mods_from_flags(nsflags::COMMAND), modifiers::COMMAND);
        assert_eq!(mods_from_flags(nsflags::CAPS_LOCK), modifiers::CAPS);
        assert_eq!(
            mods_from_flags(nsflags::CONTROL | nsflags::SHIFT),
            modifiers::CONTROL | modifiers::SHIFT
        );
    }

    #[test]
    fn named_keys_map_to_win32_vks() {
        assert_eq!(vkey_from_keycode(kvk::RETURN), vk::RETURN);
        assert_eq!(vkey_from_keycode(kvk::DELETE), vk::BACK); // Mac Delete = Backspace
        assert_eq!(vkey_from_keycode(kvk::FORWARD_DELETE), vk::DELETE);
        assert_eq!(vkey_from_keycode(kvk::ESCAPE), vk::ESCAPE);
        assert_eq!(vkey_from_keycode(kvk::LEFT), vk::LEFT);
        assert_eq!(vkey_from_keycode(kvk::F12), vk::F1 + 11);
        assert_eq!(vkey_from_keycode(0x00), 0);
    }

    #[test]
    fn printable_chars_map_to_vks() {
        assert_eq!(vkey_from_char('a'), 0x41);
        assert_eq!(vkey_from_char('A'), 0x41);
        assert_eq!(vkey_from_char('z'), 0x5A);
        assert_eq!(vkey_from_char('9'), 0x39);
        assert_eq!(vkey_from_char('!'), 0);
    }

    #[test]
    fn resolve_prefers_keycode_then_char() {
        assert_eq!(resolve_vkey(kvk::RETURN, Some('\r')), vk::RETURN);
        assert_eq!(resolve_vkey(0x00, Some('k')), 0x4B);
        assert_eq!(resolve_vkey(0x00, None), 0);
    }

    #[test]
    fn y_flips_to_top_left() {
        assert_eq!(to_top_left_y(0.0, 300.0), 300.0);
        assert_eq!(to_top_left_y(300.0, 300.0), 0.0);
    }

    #[test]
    fn builders_produce_expected_events() {
        let ev = key_event(kvk::LEFT, None, nsflags::CONTROL, true);
        assert_eq!(
            ev,
            UiEvent::Key {
                state: KeyState::Down,
                virtual_key: vk::LEFT,
                modifiers: modifiers::CONTROL,
            }
        );
        let ev = mouse_event(10.0, 20.0, MouseOp::Down, MouseButton::Left, 0, 0, 99);
        match ev {
            UiEvent::Mouse(m) => {
                assert_eq!((m.x, m.y), (10, 20));
                assert_eq!(m.op, MouseOp::Down);
                assert_eq!(m.button, MouseButton::Left);
            }
            _ => panic!("expected Mouse"),
        }
    }
}

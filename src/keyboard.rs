//! The low-level keyboard and mouse hooks that drive transliteration.
//!
//! Both procedures run on the installing thread, i.e. the message loop in
//! `main`, so they can touch [`crate::state`] directly. They must also return
//! quickly: Windows silently removes a low-level hook that exceeds
//! `LowLevelHooksTimeout` (300 ms by default). Everything expensive — resolving
//! the target process, reading the registry — happens in the WinEvent hook
//! instead.

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetKeyState, VIRTUAL_KEY, VK_CAPITAL, VK_CONTROL, VK_DECIMAL, VK_F11,
    VK_LWIN, VK_MENU, VK_OEM_1, VK_OEM_3, VK_OEM_COMMA, VK_OEM_PERIOD, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, KBDLLHOOKSTRUCT, MSLLHOOKSTRUCT, PostMessageW, WM_KEYDOWN,
    WM_KEYUP, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::WM_APP_TOGGLE;
use crate::input::{self, OKKHOR_MAGIC};
use crate::state::{self, App};

/// What a key means to the transliteration buffer.
enum Action {
    /// Extends the current word.
    Word(char),
    /// Ends the current word. Passed through untouched. Backspace is one of
    /// these: the editor has no way to unwind a word, so the key goes to the
    /// application and deletes from the screen directly.
    Break,
    /// A modifier. Passed through without ending the word — `Shift` in
    /// particular must not flush, since Avro is case sensitive (`s` is স but
    /// `S` is শ).
    Transparent,
}

fn modifier_held(vk: VIRTUAL_KEY) -> bool {
    (unsafe { GetAsyncKeyState(vk.0 as i32) } as u16 & 0x8000) != 0
}

fn caps_lock_on() -> bool {
    (unsafe { GetKeyState(VK_CAPITAL.0 as i32) } as u16 & 0x0001) != 0
}

/// Punctuation that okkhor has conversions for, and therefore has to reach the
/// parser instead of being passed through as ASCII.
///
/// | typed | becomes | |
/// |-------|---------|-|
/// | `.`   | `।`     | danda, but stays `.` in front of a digit so `3.14` works |
/// | `..`  | `।।`    | |
/// | `:`   | `ঃ`     | visarga |
/// | `^`   | `ঁ`     | candrabindu |
/// | `,,`  | `্‌`     | hasant + ZWNJ |
/// | `$`   | `৳`     | taka sign |
///
/// These extend the buffered word rather than ending it. They have to: the
/// multi-character patterns only fire when both characters sit in the same
/// buffer, and the danda rule needs to see whether a digit follows. A lone `,`
/// converts to itself, and is here only so `,,` can be recognised.
///
/// Each of these also has a backtick escape in okkhor — `` .` `` types a
/// literal `.` — which works because the backtick is buffered too.
fn punctuation(vk: u32, shift: bool) -> Option<char> {
    let vk = vk as u16;
    Some(match (vk, shift) {
        (v, false) if v == VK_OEM_PERIOD.0 => '.',
        (v, _) if v == VK_DECIMAL.0 => '.',
        (v, false) if v == VK_OEM_COMMA.0 => ',',
        (v, true) if v == VK_OEM_1.0 => ':',
        // Avro's marker for "do not combine these two letters".
        (v, false) if v == VK_OEM_3.0 => '`',
        (0x34, true) => '$',
        (0x36, true) => '^',
        _ => return None,
    })
}

/// Classify a virtual-key code.
///
/// The Latin keys are mapped by hand rather than through `ToUnicodeEx`.
/// `ToUnicodeEx` mutates the layout's dead-key state as a side effect, which
/// corrupts dead-key input in the application being typed into. Hand mapping
/// assumes a QWERTY-compatible layout for the ASCII keys, which Avro's
/// romanisation requires in any case.
fn classify(vk: u32) -> Action {
    let shift = modifier_held(VK_SHIFT);

    if let Some(symbol) = punctuation(vk, shift) {
        return Action::Word(symbol);
    }

    match vk {
        // Letters. Shift and Caps Lock combine the usual way.
        0x41..=0x5A => {
            let lower = (b'a' + (vk - 0x41) as u8) as char;
            if shift != caps_lock_on() {
                Action::Word(lower.to_ascii_uppercase())
            } else {
                Action::Word(lower)
            }
        }
        // Top-row digits become Bangla numerals. Shifted they are symbols, and
        // the two okkhor knows about were already taken by `punctuation`.
        0x30..=0x39 if !shift => Action::Word((b'0' + (vk - 0x30) as u8) as char),
        // Numeric keypad, unaffected by Shift.
        0x60..=0x69 => Action::Word((b'0' + (vk - 0x60) as u8) as char),

        v if v == VK_SHIFT.0 as u32
            || v == VK_CONTROL.0 as u32
            || v == VK_MENU.0 as u32
            || v == VK_CAPITAL.0 as u32
            || v == VK_LWIN.0 as u32
            || v == VK_RWIN.0 as u32
            || (0xA0..=0xA5).contains(&v) =>
        {
            Action::Transparent
        }

        _ => Action::Break,
    }
}

/// True when the keystroke is part of a shortcut rather than typed text.
fn is_shortcut() -> bool {
    modifier_held(VK_CONTROL)
        || modifier_held(VK_MENU)
        || modifier_held(VK_LWIN)
        || modifier_held(VK_RWIN)
}

pub unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let pass = || unsafe { CallNextHookEx(None, code, wparam, lparam) };

    if code < 0 {
        return pass();
    }

    let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };

    // Our own injected replacement text comes straight back through here.
    if info.dwExtraInfo == OKKHOR_MAGIC {
        return pass();
    }

    // `swallowed` is indexed by virtual-key code, so anything outside a byte
    // cannot be one of ours.
    if info.vkCode > 0xFF {
        return pass();
    }

    let swallow = match wparam.0 as u32 {
        WM_KEYDOWN | WM_SYSKEYDOWN => on_key_down(info.vkCode),
        WM_KEYUP | WM_SYSKEYUP => on_key_up(info.vkCode),
        _ => false,
    };

    if swallow { LRESULT(1) } else { pass() }
}

fn on_key_down(vk: u32) -> bool {
    state::with_app(|app| {
        if fallback_hotkey(app, vk) {
            return true;
        }

        let foreground = unsafe { GetForegroundWindow() };

        // Keep the cached target honest even if the WinEvent hook missed a
        // transition; a stale target would mean typing into the wrong buffer.
        if foreground != app.target {
            app.abandon_word();
            app.retarget(foreground);
        }

        if !app.is_active(foreground) || is_shortcut() {
            app.abandon_word();
            return false;
        }

        let swallow = match classify(vk) {
            Action::Word(c) => {
                let edit = app.editor.put_char(c);
                input::send_replacement(edit.backspaces, edit.output);
                // Swallow whatever the editor took, including when the edit is
                // empty: `o` after `k` leaves ক unchanged on screen but is
                // still buffered, and is what makes the next letter convert
                // correctly. Passing it on would type a literal `o`.
                //
                // Nothing `classify` produces is rejected today — the editor
                // takes any printable ASCII — but honouring the flag means such
                // a character would reach the application rather than vanish.
                !edit.pass_through
            }
            Action::Transparent => false,
            Action::Break => {
                app.abandon_word();
                false
            }
        };

        if swallow {
            app.swallowed[vk as usize] = true;
        }
        swallow
    })
    .unwrap_or(false)
}

/// Swallow the key-up of any key whose key-down we swallowed, so the target
/// never observes half a keystroke.
fn on_key_up(vk: u32) -> bool {
    state::with_app(|app| std::mem::replace(&mut app.swallowed[vk as usize], false))
        .unwrap_or(false)
}

/// Handle F11 here when `RegisterHotKey` was unavailable. Returns whether the
/// key was ours, and so must not reach the application.
fn fallback_hotkey(app: &App, vk: u32) -> bool {
    if !app.hotkeys_via_hook || vk != VK_F11.0 as u32 {
        return false;
    }
    // Posted rather than handled inline: the state is borrowed right now, and
    // the handlers need it too.
    unsafe { PostMessageW(Some(app.msg_hwnd), WM_APP_TOGGLE, WPARAM(0), LPARAM(0)).ok() };
    true
}

/// Any click moves the caret somewhere we cannot predict, so the buffered word
/// is abandoned. Clicks themselves are never swallowed.
pub unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        if info.dwExtraInfo != OKKHOR_MAGIC
            && matches!(
                wparam.0 as u32,
                WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN
            )
        {
            state::with_app(|app| app.abandon_word());
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Tests for the assumptions `classify` makes about okkhor.
///
/// The live preview itself belongs to `okkhor::editor` and is tested there.
/// What is left here is the contract between the two: which keys this module
/// feeds into the buffer, and why it must not break a word where it might look
/// safe to.
#[cfg(test)]
mod tests {
    use super::*;
    use okkhor::parser::Parser;

    /// Every symbol routed into the buffer has to be one okkhor converts, and
    /// has to survive `put_char`, which drops anything outside printable ASCII.
    ///
    /// The failure is silent in both directions: route a key okkhor cannot use
    /// and the raw ASCII sits in the buffer; fail to route one it can and
    /// `classify` breaks the word, losing the conversion.
    #[test]
    fn routed_symbols_are_the_ones_okkhor_converts() {
        for (vk, shift, ch) in [
            (VK_OEM_PERIOD.0 as u32, false, '.'),
            (VK_DECIMAL.0 as u32, false, '.'),
            (VK_OEM_COMMA.0 as u32, false, ','),
            (VK_OEM_1.0 as u32, true, ':'),
            (VK_OEM_3.0 as u32, false, '`'),
            (0x34, true, '$'),
            (0x36, true, '^'),
        ] {
            assert_eq!(punctuation(vk, shift), Some(ch), "vk {vk:#04X}");
            assert!(ch.is_ascii_graphic(), "put_char would drop {ch:?}");
        }

        let parser = Parser::new_phonetic();
        assert_eq!(parser.convert("."), "\u{0964}"); // danda ।
        assert_eq!(parser.convert(".."), "\u{0964}\u{0964}"); // ।।
        assert_eq!(parser.convert(":"), "\u{0983}"); // visarga ঃ
        assert_eq!(parser.convert("^"), "\u{0981}"); // candrabindu ঁ
        assert_eq!(parser.convert(",,"), "\u{09CD}\u{200C}"); // hasant + ZWNJ
        assert_eq!(parser.convert("$"), "\u{09F3}"); // taka ৳
        // The backtick escapes each of them, which only works because it is
        // buffered alongside rather than treated as a word break.
        assert_eq!(parser.convert(".`"), ".");
    }

    /// Why Shift is `Transparent` rather than a word break: Avro uses case to
    /// pick a different consonant, not a stylistic variant.
    #[test]
    fn case_selects_different_consonants() {
        let parser = Parser::new_phonetic();
        assert_eq!(parser.convert("s"), "স");
        assert_eq!(parser.convert("S"), "শ");
        assert_eq!(parser.convert("t"), "ত");
        assert_eq!(parser.convert("T"), "ট");
        assert_eq!(parser.convert("Dhaka"), "ঢাকা");
    }

    /// Why a word may not be broken between letters: `x` means one thing alone
    /// and another after `k`. Splitting the two is what the dropped-buffer bug
    /// looked like, and what `scripts/e2e-focus-noise.ps1` pins on the desktop.
    #[test]
    fn later_letters_reinterpret_earlier_ones() {
        let parser = Parser::new_phonetic();
        assert_eq!(parser.convert("kx"), "ক্ষ");
        assert_eq!(parser.convert("kxoma"), "ক্ষমা");
        assert_eq!(
            format!("{}{}", parser.convert("k"), parser.convert("x")),
            "কএক্স"
        );
    }

    /// Why digits extend a word instead of ending it, and why a dot has to stay
    /// buffered: the danda it produces becomes a plain dot once a digit follows.
    #[test]
    fn digits_extend_the_word() {
        let parser = Parser::new_phonetic();
        assert_eq!(parser.convert("123"), "১২৩");
        assert_eq!(parser.convert("3."), "৩\u{0964}");
        assert_eq!(parser.convert("3.14"), "৩.১৪");
    }
}

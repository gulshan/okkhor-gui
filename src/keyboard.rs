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
    GetAsyncKeyState, GetKeyState, VK_BACK, VK_CAPITAL, VK_CONTROL, VK_DECIMAL, VK_F11, VK_F12,
    VK_LWIN, VK_MENU, VK_OEM_1, VK_OEM_3, VK_OEM_COMMA, VK_OEM_PERIOD, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetForegroundWindow, KBDLLHOOKSTRUCT, MSLLHOOKSTRUCT, PostMessageW, WM_KEYDOWN,
    WM_KEYUP, WM_LBUTTONDOWN, WM_MBUTTONDOWN, WM_RBUTTONDOWN, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use crate::input::OKKHOR_MAGIC;
use crate::state::{self, App};
use crate::{WM_APP_QUIT, WM_APP_TOGGLE};

/// What a key means to the transliteration buffer.
enum Action {
    /// Extends the current word.
    Word(char),
    /// Removes the last character of the current word.
    Erase,
    /// Ends the current word. Passed through untouched.
    Break,
    /// A modifier. Passed through without ending the word — `Shift` in
    /// particular must not flush, since Avro is case sensitive (`s` is স but
    /// `S` is শ).
    Transparent,
}

fn modifier_held(vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY) -> bool {
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

        v if v == VK_BACK.0 as u32 => Action::Erase,

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

    let swallow = match wparam.0 as u32 {
        WM_KEYDOWN | WM_SYSKEYDOWN => on_key_down(info.vkCode),
        WM_KEYUP | WM_SYSKEYUP => on_key_up(info.vkCode),
        _ => false,
    };

    if swallow { LRESULT(1) } else { pass() }
}

fn on_key_down(vk: u32) -> bool {
    if vk > 0xFF {
        return false;
    }

    state::with_app(|app| {
        if let Some(swallow) = fallback_hotkey(app, vk) {
            return swallow;
        }

        let foreground = unsafe { GetForegroundWindow() };

        // Keep the cached target honest even if the WinEvent hook missed a
        // transition; a stale target would mean typing into the wrong buffer.
        if foreground != app.target {
            app.session.clear();
            app.retarget(foreground);
        }

        if !app.is_active(foreground) {
            app.session.clear();
            return false;
        }

        if is_shortcut() {
            app.session.clear();
            return false;
        }

        let swallow = match classify(vk) {
            Action::Word(c) => {
                app.session.raw.push(c);
                app.render();
                true
            }
            Action::Erase if !app.session.raw.is_empty() => {
                app.session.raw.pop();
                app.render();
                true
            }
            Action::Transparent => false,
            // A backspace with nothing buffered, or any word-breaking key,
            // ends the word and reaches the application unchanged.
            Action::Erase | Action::Break => {
                app.session.clear();
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
    if vk > 0xFF {
        return false;
    }
    state::with_app(|app| std::mem::replace(&mut app.swallowed[vk as usize], false))
        .unwrap_or(false)
}

/// Handle F11/F12 here when `RegisterHotKey` was unavailable. Returns `None`
/// when the key is not one of ours.
fn fallback_hotkey(app: &App, vk: u32) -> Option<bool> {
    if !app.hotkeys_via_hook {
        return None;
    }
    let message = match vk {
        v if v == VK_F11.0 as u32 => WM_APP_TOGGLE,
        v if v == VK_F12.0 as u32 => WM_APP_QUIT,
        _ => return None,
    };
    // Posted rather than handled inline: the state is borrowed right now, and
    // the handlers need it too.
    unsafe { PostMessageW(Some(app.msg_hwnd), message, WPARAM(0), LPARAM(0)).ok() };
    Some(true)
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
            state::with_app(|app| app.session.clear());
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

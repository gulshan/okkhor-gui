//! Synthetic keystroke injection.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, SendInput, VIRTUAL_KEY, VK_BACK,
};

/// Stamped into `dwExtraInfo` of everything we inject so the keyboard hook can
/// recognise its own echo. More reliable than testing `LLKHF_INJECTED`, which
/// is also set by any other tool driving the keyboard.
pub const OKKHOR_MAGIC: usize = 0x4F4B_4B48; // "OKKH"

/// One keyboard event. The two kinds sent from here fill in opposite fields: a
/// virtual key carries no scan code, and a `KEYEVENTF_UNICODE` event carries a
/// UTF-16 unit in `wScan` and no virtual key.
fn key_event(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: OKKHOR_MAGIC,
            },
        },
    }
}

/// Erase `backspaces` units, then type `text`.
///
/// Sent as a single `SendInput` call so the whole replacement enters the
/// target's input queue atomically — a competing keystroke can never land
/// between the erase and the retype.
pub fn send_replacement(backspaces: usize, text: &str) {
    let units: Vec<u16> = text.encode_utf16().collect();
    if backspaces == 0 && units.is_empty() {
        return;
    }

    let mut events = Vec::with_capacity(backspaces * 2 + units.len() * 2);
    for _ in 0..backspaces {
        events.push(key_event(VK_BACK, 0, KEYBD_EVENT_FLAGS(0)));
        events.push(key_event(VK_BACK, 0, KEYEVENTF_KEYUP));
    }
    for unit in units {
        events.push(key_event(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE));
        events.push(key_event(
            VIRTUAL_KEY(0),
            unit,
            KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
        ));
    }

    unsafe {
        SendInput(&events, size_of::<INPUT>() as i32);
    }
}

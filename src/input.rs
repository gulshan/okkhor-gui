//! Synthetic keystroke injection.

use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_UNICODE, SendInput, VIRTUAL_KEY, VK_BACK,
};

/// Stamped into `dwExtraInfo` of everything we inject so the keyboard hook can
/// recognise its own echo. More reliable than testing `LLKHF_INJECTED`, which
/// is also set by any other tool driving the keyboard.
pub const OKKHOR_MAGIC: usize = 0x4F4B_4B48; // "OKKH"

fn vk_event(vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: OKKHOR_MAGIC,
            },
        },
    }
}

fn unicode_event(unit: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(0),
                wScan: unit,
                dwFlags: KEYEVENTF_UNICODE | flags,
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
        events.push(vk_event(VK_BACK, KEYBD_EVENT_FLAGS(0)));
        events.push(vk_event(VK_BACK, KEYEVENTF_KEYUP));
    }
    for unit in units {
        events.push(unicode_event(unit, KEYBD_EVENT_FLAGS(0)));
        events.push(unicode_event(unit, KEYEVENTF_KEYUP));
    }

    unsafe {
        SendInput(&events, size_of::<INPUT>() as i32);
    }
}

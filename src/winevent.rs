//! Reacting to focus movement.
//!
//! Two things happen here. The typing buffer is abandoned whenever focus moves,
//! because the caret is no longer where we left it. And the comparatively
//! expensive work of resolving the foreground process is done here rather than
//! in the keyboard hook, so it stays off the per-keystroke path.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows::Win32::UI::WindowsAndMessaging::{
    CHILDID_SELF, EVENT_OBJECT_DESTROY, EVENT_OBJECT_FOCUS, EVENT_SYSTEM_FOREGROUND, GA_ROOT,
    GetAncestor, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
};

use crate::state::{self, key_of};
use crate::tray;

/// Install the focus/destroy listeners. Returns the hooks so they can be
/// unhooked on exit.
pub fn install() -> [HWINEVENTHOOK; 2] {
    let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
    unsafe {
        [
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(win_event_proc),
                0,
                0,
                flags,
            ),
            SetWinEventHook(
                EVENT_OBJECT_DESTROY,
                EVENT_OBJECT_FOCUS,
                None,
                Some(win_event_proc),
                0,
                0,
                flags,
            ),
        ]
    }
}

unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _thread: u32,
    _time: u32,
) {
    match event {
        EVENT_SYSTEM_FOREGROUND if id_object == OBJID_WINDOW.0 => {
            // Clicking the notification area momentarily makes an explorer
            // window the foreground one. Adopting it as the target would point
            // the tray menu at the wrong application.
            if state::is_shell_window(hwnd) {
                return;
            }
            state::with_app(|app| {
                app.abandon_word();
                app.retarget(hwnd);
            });
            tray::refresh();
        }
        EVENT_OBJECT_FOCUS => {
            // Only a focus move inside the window being typed into means the
            // caret has gone somewhere unexpected.
            //
            // These events are noisy and global: applications announce their
            // internal focus changes with NotifyWinEvent, and every one of them
            // reaches this hook regardless of which process it came from.
            // Clearing on all of them abandoned the buffered word at random
            // while the user was still typing it, which corrupts any spelling
            // whose meaning depends on earlier letters — `kx` came out as
            // কএক্স instead of ক্ষ, because the k was committed alone and the x
            // then re-analysed as a fresh word.
            let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
            state::with_app(|app| {
                if !app.target.is_invalid() && (root == app.target || hwnd == app.target) {
                    app.abandon_word();
                }
            });
        }
        EVENT_OBJECT_DESTROY if id_object == OBJID_WINDOW.0 && id_child == CHILDID_SELF as i32 => {
            state::with_app(|app| {
                app.modes.remove(&key_of(hwnd));
            });
        }
        _ => {}
    }
}

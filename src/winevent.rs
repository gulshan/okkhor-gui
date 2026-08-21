//! Window focus and lifetime event listeners.

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
            // A *child* control of the window being typed into took focus, so
            // the caret has gone somewhere we cannot predict.
            //
            // Two filters, each paid for by a bug.
            //
            // The event has to belong to the target window at all. These events
            // are noisy and global: applications announce their internal focus
            // changes with NotifyWinEvent, and every one reaches this hook
            // whatever process it came from. Clearing on all of them abandoned
            // the word at random while it was still being typed, which corrupts
            // any spelling whose meaning depends on an earlier letter — `kx`
            // came out as কএক্স instead of ক্ষ.
            //
            // And the window may not be announcing focus on *itself*. Chromium
            // does that on its own top-level HWND for every keystroke into the
            // address bar, which abandoned the word between letters: `ami` came
            // out as আমই, with the `i` a standalone vowel instead of a sign on
            // the ম. A real caret move lands on a child control, so `hwnd` is
            // below `root` rather than equal to it, which separates the two.
            //
            // What this no longer catches is a purely programmatic focus move
            // that reports the top-level window. Nothing is lost in practice:
            // a click is caught by the mouse hook, Tab and the arrow keys are
            // word breaks already, and switching window is caught by the
            // foreground check in `keyboard`.
            let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
            state::with_app(|app| {
                if !app.target.is_invalid() && root == app.target && hwnd != root {
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

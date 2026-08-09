//! Shared program state.
//!
//! Every callback in this program — the two low-level hooks, the WinEvent
//! hook and the window procedure — is invoked on the thread that installed
//! them, which is the single thread in `main`. That makes a `thread_local!`
//! with plain `RefCell` interior mutability both sufficient and correct; no
//! locking is involved anywhere.
//!
//! The one rule to respect: never hold a borrow across a call that can pump
//! messages (`TrackPopupMenu` in particular), or the re-entrant callback will
//! panic on the outstanding borrow.

use std::cell::RefCell;
use std::collections::HashMap;

use okkhor::parser::Parser;
use windows::Win32::Foundation::{CloseHandle, HWND, MAX_PATH};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetWindowThreadProcessId, IsWindow};
use windows::core::PWSTR;

use crate::input;

/// The romanised text typed so far and the Bangla currently showing for it.
#[derive(Default)]
pub struct Session {
    pub raw: String,
    pub emitted: String,
}

impl Session {
    /// Forget the current word without touching the screen. Used whenever we
    /// lose track of where the caret is (focus change, mouse click, arrow
    /// key), so a desync can never propagate past one word.
    pub fn clear(&mut self) {
        self.raw.clear();
        self.emitted.clear();
    }
}

pub struct App {
    pub parser: Parser,
    /// Per top-level window: is transliteration active? Absent means inactive,
    /// which is the required default.
    pub modes: HashMap<isize, bool>,
    pub session: Session,
    /// The window keystrokes are going to. Cached by the WinEvent hook so the
    /// tray menu still knows the real target after the shell steals focus.
    pub target: HWND,
    /// Full image path of `target`'s process, resolved off the hot path.
    pub target_exe: String,
    /// Our own message-only window, used by callbacks that need to defer work
    /// back to the message loop.
    pub msg_hwnd: HWND,
    /// Set when `RegisterHotKey` failed and the keyboard hook has to detect
    /// F11/F12 itself.
    pub hotkeys_via_hook: bool,
    /// Keys whose key-down we swallowed, so we can swallow the matching key-up
    /// and not leave the target seeing half a keystroke.
    pub swallowed: [bool; 256],
}

impl App {
    fn new() -> Self {
        App {
            parser: Parser::new_phonetic(),
            modes: HashMap::new(),
            session: Session::default(),
            target: HWND(std::ptr::null_mut()),
            target_exe: String::new(),
            msg_hwnd: HWND(std::ptr::null_mut()),
            hotkeys_via_hook: false,
            swallowed: [false; 256],
        }
    }

    pub fn is_active(&self, hwnd: HWND) -> bool {
        self.modes.get(&key_of(hwnd)).copied().unwrap_or(false)
    }

    /// Flip the mode of `hwnd` and return the new value.
    pub fn toggle(&mut self, hwnd: HWND) -> bool {
        self.session.clear();
        let flag = self.modes.entry(key_of(hwnd)).or_insert(false);
        *flag = !*flag;
        let now = *flag;
        self.prune();
        now
    }

    /// Drop entries for windows that no longer exist. Window handles are
    /// recycled by Windows, so leaving stale entries around would eventually
    /// hand a fresh window somebody else's mode.
    pub fn prune(&mut self) {
        self.modes
            .retain(|&raw, _| unsafe { IsWindow(Some(hwnd_of(raw))).as_bool() });
    }

    /// Re-resolve the cached target and everything derived from it. Called
    /// from the WinEvent hook, never from the keyboard hook.
    pub fn retarget(&mut self, hwnd: HWND) {
        self.target = hwnd;
        self.target_exe = exe_path_of(hwnd);
    }

    /// Re-convert the buffer and patch the difference onto the screen.
    pub fn render(&mut self) {
        let next = self.parser.convert(&self.session.raw);
        let replacement = crate::bengali::diff(&self.session.emitted, &next);
        input::send_replacement(replacement.backspaces, &replacement.text);
        self.session.emitted = next;
    }
}

thread_local! {
    static APP: RefCell<App> = RefCell::new(App::new());
}

/// Run `f` with mutable access to the program state.
///
/// Returns `None` if the state is already borrowed. That only happens if a
/// callback re-enters while another is mid-flight; dropping the event is far
/// better than panicking inside a hook procedure, which would take the hook
/// down with it.
pub fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|cell| cell.try_borrow_mut().ok().map(|mut app| f(&mut app)))
}

pub fn key_of(hwnd: HWND) -> isize {
    hwnd.0 as isize
}

fn hwnd_of(key: isize) -> HWND {
    HWND(key as *mut core::ffi::c_void)
}

/// Window classes that own the notification area. A click on the tray briefly
/// makes one of these the foreground window; treating that as the new
/// transliteration target would make the tray menu act on the wrong window.
const SHELL_CLASSES: [&str; 5] = [
    "Shell_TrayWnd",
    "NotifyIconOverflowWindow",
    "TopLevelWindowForOverflowXamlIsland",
    "Windows.UI.Core.CoreWindow",
    "XamlExplorerHostIslandWindow",
];

pub fn is_shell_window(hwnd: HWND) -> bool {
    let mut buffer = [0u16; 256];
    let written = unsafe { GetClassNameW(hwnd, &mut buffer) };
    if written <= 0 {
        return false;
    }
    let class = String::from_utf16_lossy(&buffer[..written as usize]);
    SHELL_CLASSES.contains(&class.as_str())
}

/// Full image path of the process owning `hwnd`, or an empty string.
pub fn exe_path_of(hwnd: HWND) -> String {
    if hwnd.is_invalid() {
        return String::new();
    }

    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 {
        return String::new();
    }

    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return String::new();
        };

        let mut buffer = [0u16; MAX_PATH as usize];
        let mut length = buffer.len() as u32;
        let path = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
        .map(|()| String::from_utf16_lossy(&buffer[..length as usize]))
        .unwrap_or_default();

        let _ = CloseHandle(process);
        path
    }
}

/// Just the file name, for display.
pub fn exe_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

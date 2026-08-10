//! okkhor-gui — background Bangla phonetic input for Windows.
//!
//! Runs headless with a notification-area icon. Each top-level window carries
//! its own active/inactive mode, defaulting to inactive; F11 toggles the mode
//! of the focused window, and quitting is done from the tray menu. While a
//! window is active, romanised
//! keystrokes are swallowed, buffered, converted with the `okkhor` crate and
//! re-emitted as Bangla with `SendInput`.

#![windows_subsystem = "windows"]

mod autostart;
mod bengali;
mod input;
mod keyboard;
mod setup;
mod state;
mod tray;
mod winevent;

use windows::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::Accessibility::UnhookWinEvent;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey, VK_F11,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetForegroundWindow, GetMessageW, MSG,
    PostQuitMessage, RegisterClassW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP, WM_DESTROY, WM_HOTKEY, WM_LBUTTONUP, WM_RBUTTONUP,
    WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::w;

/// Notification-area callback message.
pub const WM_APP_TRAY: u32 = WM_APP + 1;
/// Posted by the keyboard hook when it has to stand in for `RegisterHotKey`.
pub const WM_APP_TOGGLE: u32 = WM_APP + 2;
/// Posted by the tray menu, or by the installer asking a running instance to
/// step aside, to shut down.
pub const WM_APP_QUIT: u32 = WM_APP + 3;

const HOTKEY_TOGGLE: i32 = 1;

fn main() {
    // Install and uninstall run before the singleton guard: they are
    // short-lived helper invocations, and they need to be able to run while an
    // instance of the app is up so they can ask it to quit.
    let mode = match setup::parse_args() {
        setup::Mode::Offer => setup::offer(false),
        other => other,
    };

    match mode {
        setup::Mode::Install { silent } => std::process::exit(setup::install(silent)),
        setup::Mode::Uninstall { silent } => std::process::exit(setup::uninstall(silent)),
        setup::Mode::FinishUninstall { dir, pid } => {
            std::process::exit(setup::finish_uninstall(&dir, pid))
        }
        setup::Mode::Run | setup::Mode::Offer => {}
    }

    run_tray_app();
}

fn run_tray_app() {
    // A second instance would install a second set of hooks and every
    // keystroke would be transliterated twice.
    let _singleton = unsafe { CreateMutexW(None, true, w!("Local\\okkhor-gui-singleton")) };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return;
    }

    let instance = unsafe { GetModuleHandleW(None) }.expect("module handle");

    let class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance.into(),
        lpszClassName: w!("okkhor-gui.window"),
        ..Default::default()
    };
    unsafe { RegisterClassW(&class) };

    // A zero-sized window that is never shown. It owns the hotkey
    // registrations and the tray callback.
    //
    // Deliberately *not* an `HWND_MESSAGE` window: message-only windows cannot
    // become the foreground window, and `TrackPopupMenu` needs its owner in the
    // foreground or the menu will not close when the user clicks away.
    // `WS_EX_TOOLWINDOW` plus never calling `ShowWindow` keeps it out of the
    // taskbar and out of Alt+Tab.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("okkhor-gui.window"),
            w!("okkhor-gui"),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )
    }
    .expect("message window");

    state::with_app(|app| app.msg_hwnd = hwnd);

    let keyboard =
        unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard::keyboard_hook), None, 0) }
            .expect("keyboard hook");
    let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(keyboard::mouse_hook), None, 0) }.ok();
    let win_events = winevent::install();

    register_hotkey(hwnd);

    // Seed the target so the tray shows something sensible before the first
    // focus change.
    let foreground = unsafe { GetForegroundWindow() };
    state::with_app(|app| app.retarget(foreground));

    tray::install(hwnd);

    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    tray::remove();
    unsafe {
        let _ = UnregisterHotKey(Some(hwnd), HOTKEY_TOGGLE);
        for hook in win_events {
            let _ = UnhookWinEvent(hook);
        }
        if let Some(mouse) = mouse {
            let _ = UnhookWindowsHookEx(mouse);
        }
        let _ = UnhookWindowsHookEx(keyboard);
    }
}

/// Claim F11 globally, falling back to detecting it in the keyboard hook if the
/// registration is refused — only one application at a time can hold a hotkey,
/// so another one may already have it.
fn register_hotkey(hwnd: HWND) {
    let registered =
        unsafe { RegisterHotKey(Some(hwnd), HOTKEY_TOGGLE, MOD_NOREPEAT, VK_F11.0 as u32) };
    if registered.is_err() {
        state::with_app(|app| app.hotkeys_via_hook = true);
    }
}

fn toggle_foreground() {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return;
    }
    state::with_app(|app| {
        if hwnd != app.target {
            app.retarget(hwnd);
        }
        app.toggle(hwnd);
    });
    tray::refresh();
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_HOTKEY => {
            if wparam.0 as i32 == HOTKEY_TOGGLE {
                toggle_foreground();
            }
            LRESULT(0)
        }
        WM_APP_TOGGLE => {
            toggle_foreground();
            LRESULT(0)
        }
        WM_APP_QUIT => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        WM_APP_TRAY => {
            if matches!(lparam.0 as u32, WM_RBUTTONUP | WM_LBUTTONUP) {
                tray::show_menu(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

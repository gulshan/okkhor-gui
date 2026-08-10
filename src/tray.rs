//! Notification-area icon and its context menu.
//!
//! The two icons are drawn with GDI at startup rather than shipped as `.ico`
//! resources, which keeps the program a single self-contained executable
//! without pulling in a resource-compiler build dependency.

use std::cell::RefCell;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CLIP_DEFAULT_PRECIS, CreateBitmap, CreateCompatibleDC,
    CreateDIBSection, CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_PITCH,
    DEFAULT_QUALITY, DIB_RGB_COLORS, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject,
    DrawTextW, FW_SEMIBOLD, FillRect, HBITMAP, HGDIOBJ, OUT_DEFAULT_PRECIS, SelectObject,
    SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW,
    Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, DestroyIcon, DestroyMenu, GetCursorPos,
    HICON, ICONINFO, MF_CHECKED, MF_SEPARATOR, MF_STRING, PostMessageW, SetForegroundWindow,
    TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, WM_NULL,
};
use windows::core::{PCWSTR, w};

use crate::autostart::wide;
use crate::state;
use crate::{WM_APP_QUIT, WM_APP_TRAY};

const TRAY_ID: u32 = 1;

const ICON_SIZE: i32 = 32;
const ACTIVE_BACKGROUND: u32 = 0x00_3E_8E_1E; // 0x00BBGGRR — green
const IDLE_BACKGROUND: u32 = 0x00_68_63_5F; // grey

const CMD_TOGGLE: usize = 1;
const CMD_AUTOSTART: usize = 2;
const CMD_INSTALL: usize = 3;
const CMD_EXIT: usize = 4;

struct Tray {
    hwnd: HWND,
    active: HICON,
    idle: HICON,
}

thread_local! {
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

/// A square filled with `background` and a single glyph centred on it, as
/// top-down BGRA pixels.
///
/// Kept separate from icon creation so the installer can write the very same
/// artwork out as a real `.ico` file, rather than carrying a second copy of the
/// drawing code. See [`crate::setup`].
fn draw_glyph(size: i32, background: u32, glyph: &str) -> Vec<u8> {
    let mut pixels = vec![0u8; (size * size * 4) as usize];

    unsafe {
        let dc = CreateCompatibleDC(None);

        let mut info = BITMAPINFO::default();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = size;
        info.bmiHeader.biHeight = -size; // negative: top-down rows
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB.0;

        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        else {
            let _ = DeleteDC(dc);
            return pixels;
        };

        let previous_bitmap = SelectObject(dc, HGDIOBJ::from(bitmap));

        let brush = CreateSolidBrush(COLORREF(background));
        let area = RECT {
            left: 0,
            top: 0,
            right: size,
            bottom: size,
        };
        FillRect(dc, &area, brush);
        let _ = DeleteObject(HGDIOBJ::from(brush));

        // Nirmala UI is the stock Windows font with Bangla coverage. If it is
        // missing GDI substitutes something else rather than failing.
        let font = CreateFontW(
            -(size * 22 / 32),
            0,
            0,
            0,
            FW_SEMIBOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            DEFAULT_QUALITY,
            DEFAULT_PITCH.0 as u32,
            w!("Nirmala UI"),
        );
        let previous_font = SelectObject(dc, HGDIOBJ::from(font));

        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00FF_FFFF));

        let mut text: Vec<u16> = glyph.encode_utf16().collect();
        let mut text_area = area;
        DrawTextW(
            dc,
            &mut text,
            &mut text_area,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );

        // GDI text and fill leave the alpha channel at zero, which would make
        // the whole image transparent. Force every pixel opaque.
        let drawn = std::slice::from_raw_parts(bits as *const u8, pixels.len());
        pixels.copy_from_slice(drawn);
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = 0xFF;
        }

        SelectObject(dc, previous_font);
        SelectObject(dc, previous_bitmap);
        let _ = DeleteObject(HGDIOBJ::from(font));
        let _ = DeleteObject(HGDIOBJ::from(bitmap));
        let _ = DeleteDC(dc);
    }

    pixels
}

/// Wrap top-down BGRA pixels in an `HICON`.
fn icon_from_pixels(size: i32, pixels: &[u8]) -> HICON {
    unsafe {
        let mut info = BITMAPINFO::default();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = size;
        info.bmiHeader.biHeight = -size;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB.0;

        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(colour) = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0) else {
            return HICON(std::ptr::null_mut());
        };
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), bits as *mut u8, pixels.len());

        // A 1bpp all-zero mask: with a real alpha channel the mask is unused,
        // but ICONINFO still requires one.
        let mask_bits = vec![0u8; (size * size / 8) as usize];
        let mask: HBITMAP = CreateBitmap(
            size,
            size,
            1,
            1,
            Some(mask_bits.as_ptr() as *const core::ffi::c_void),
        );

        let icon_info = ICONINFO {
            fIcon: true.into(),
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: colour,
        };
        let icon = CreateIconIndirect(&icon_info).unwrap_or(HICON(std::ptr::null_mut()));

        let _ = DeleteObject(HGDIOBJ::from(mask));
        let _ = DeleteObject(HGDIOBJ::from(colour));
        icon
    }
}

fn make_icon(background: u32, glyph: &str) -> HICON {
    icon_from_pixels(ICON_SIZE, &draw_glyph(ICON_SIZE, background, glyph))
}

/// The app's identity artwork at an arbitrary size — the active tray icon,
/// used by the installer for the Start Menu shortcut and Apps & Features.
pub fn app_icon_pixels(size: i32) -> Vec<u8> {
    draw_glyph(size, ACTIVE_BACKGROUND, "অ")
}

fn base_data(hwnd: HWND) -> NOTIFYICONDATAW {
    NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_APP_TRAY,
        ..Default::default()
    }
}

pub fn install(hwnd: HWND) {
    let tray = Tray {
        hwnd,
        active: make_icon(ACTIVE_BACKGROUND, "অ"),
        idle: make_icon(IDLE_BACKGROUND, "A"),
    };

    let mut data = base_data(hwnd);
    data.hIcon = tray.idle;
    write_tip(&mut data.szTip, "okkhor-gui");
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &data);
    };

    TRAY.with(|cell| *cell.borrow_mut() = Some(tray));
    refresh();
}

/// Update the icon and tooltip to match the current target window.
pub fn refresh() {
    let Some((active, label)) = state::with_app(|app| {
        let active = app.is_active(app.target);
        let name = state::exe_name(&app.target_exe);
        let label = if name.is_empty() {
            "okkhor-gui".to_string()
        } else {
            format!("{name} — {}", if active { "ACTIVE" } else { "inactive" })
        };
        (active, label)
    }) else {
        return;
    };

    TRAY.with(|cell| {
        let borrowed = cell.borrow();
        let Some(tray) = borrowed.as_ref() else {
            return;
        };
        let mut data = base_data(tray.hwnd);
        data.hIcon = if active { tray.active } else { tray.idle };
        write_tip(&mut data.szTip, &label);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        };
    });
}

pub fn remove() {
    TRAY.with(|cell| {
        let Some(tray) = cell.borrow_mut().take() else {
            return;
        };
        let data = base_data(tray.hwnd);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            let _ = DestroyIcon(tray.active);
            let _ = DestroyIcon(tray.idle);
        }
    });
}

fn write_tip(field: &mut [u16; 128], text: &str) {
    let encoded: Vec<u16> = text.encode_utf16().take(field.len() - 1).collect();
    field[..encoded.len()].copy_from_slice(&encoded);
    field[encoded.len()] = 0;
}

/// Show the context menu and act on the selection.
///
/// `TPM_RETURNCMD` makes `TrackPopupMenu` return the chosen command instead of
/// posting `WM_COMMAND`. That matters here: the menu runs its own modal message
/// loop, so anything dispatched from inside it would re-enter the callbacks
/// while state is borrowed.
pub fn show_menu(hwnd: HWND) {
    let Some((active, autostart_on)) =
        state::with_app(|app| (app.is_active(app.target), crate::autostart::is_enabled()))
    else {
        return;
    };

    let checked = |on: bool| {
        if on {
            MF_STRING | MF_CHECKED
        } else {
            MF_STRING
        }
    };

    let command = unsafe {
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };

        let toggle_text = wide("Active for this window\tF11");
        let autostart_text = wide("Start with Windows");
        let exit_text = wide("Exit\tF12");

        let _ = AppendMenuW(
            menu,
            checked(active),
            CMD_TOGGLE,
            PCWSTR(toggle_text.as_ptr()),
        );
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(
            menu,
            checked(autostart_on),
            CMD_AUTOSTART,
            PCWSTR(autostart_text.as_ptr()),
        );
        // Only meaningful while running from outside the install directory,
        // i.e. straight out of a downloads folder.
        if !crate::setup::running_from_install_dir() {
            let install_text = wide("Install okkhor-gui on this PC…");
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, CMD_INSTALL, PCWSTR(install_text.as_ptr()));
        }

        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, CMD_EXIT, PCWSTR(exit_text.as_ptr()));

        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);

        // Without this the menu refuses to close when the user clicks away.
        let _ = SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_RETURNCMD,
            cursor.x,
            cursor.y,
            None,
            hwnd,
            None,
        );
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);

        chosen.0 as usize
    };

    match command {
        CMD_TOGGLE => {
            state::with_app(|app| {
                let target = app.target;
                app.toggle(target);
            });
            refresh();
        }
        CMD_AUTOSTART => crate::autostart::set_enabled(!autostart_on),
        CMD_INSTALL => {
            // Run the install in a fresh process and step aside, so it is free
            // to overwrite the executable this one is running from.
            let _ = std::process::Command::new(crate::autostart::own_exe_path())
                .arg("--install")
                .spawn();
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_APP_QUIT, WPARAM(0), LPARAM(0));
            }
        }
        CMD_EXIT => unsafe {
            let _ = PostMessageW(Some(hwnd), WM_APP_QUIT, WPARAM(0), LPARAM(0));
        },
        _ => {}
    }
}

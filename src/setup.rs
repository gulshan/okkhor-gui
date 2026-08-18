//! Per-user install and uninstall.
//!
//! The binary is its own installer. Nothing here ever needs administrator
//! rights: the app lands in `%LOCALAPPDATA%`, everything it registers lives
//! under `HKEY_CURRENT_USER`, and it is only ever installed for the user who
//! ran it. That matches how the app works — it hooks one interactive session
//! and cannot be shared between users anyway.
//!
//! There is no console (`windows_subsystem = "windows"`), so progress and
//! failures are reported with `MessageBoxW` unless `--silent` is passed.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{CloseHandle, LPARAM, WPARAM};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize, IPersistFile,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, IDYES, IsWindow, MB_ICONERROR, MB_ICONINFORMATION, MB_ICONQUESTION, MB_OK,
    MB_YESNO, MESSAGEBOX_STYLE, MessageBoxW, PostMessageW,
};
use windows::core::{HSTRING, Interface, PCWSTR, w};

use crate::WM_APP_QUIT;
use crate::autostart::{self, Key};

/// What the user sees: dialog captions, the Apps & Features entry, the tray.
pub const APP_NAME: &str = "অক্ষর";

/// The same name in ASCII, for the Start Menu entry.
///
/// Start Menu search matches what you type, and the people who want this app
/// are by definition typing romanised Bangla on an English keyboard — a Bangla
/// file name there would be unfindable without the app already running.
const APP_NAME_ASCII: &str = "Okkhor";

/// Stem for everything on disk and in the registry, where a name is an
/// identifier rather than something to read.
const APP_ID: &str = "okkhor";

const PUBLISHER: &str = "Gulshanur Rahman";
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Class of the hidden window that owns the hotkey and the tray icon. Also how
/// [`stop_running_instance`] finds an instance that is already up.
pub const WINDOW_CLASS: PCWSTR = w!("okkhor.window");

const UNINSTALL_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\okkhor");

/// What this invocation is supposed to do.
pub enum Mode {
    /// Run as the tray application.
    Run,
    /// Running from outside the install directory: ask whether to install.
    Offer,
    Install {
        silent: bool,
    },
    Uninstall {
        silent: bool,
    },
    /// Internal. A copy running from `%TEMP%`, waiting to delete the install
    /// directory once the uninstaller that spawned it has exited.
    FinishUninstall {
        dir: PathBuf,
        pid: u32,
    },
}

pub fn parse_args() -> Mode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let silent = args.iter().any(|a| a == "--silent");
    let has = |flag: &str| args.iter().any(|a| a == flag);

    if has("--finish-uninstall") {
        // --finish-uninstall <dir> <pid>
        let mut rest = args
            .iter()
            .skip_while(|a| *a != "--finish-uninstall")
            .skip(1);
        if let (Some(dir), Some(pid)) = (rest.next(), rest.next())
            && let Ok(pid) = pid.parse()
        {
            return Mode::FinishUninstall {
                dir: PathBuf::from(dir),
                pid,
            };
        }
        return Mode::Run;
    }

    if has("--install") {
        return Mode::Install { silent };
    }
    if has("--uninstall") {
        return Mode::Uninstall { silent };
    }
    if has("--portable") {
        return Mode::Run;
    }

    // No flags: behave like the app when installed, like a setup program when
    // launched from anywhere else (a downloads folder, a USB stick).
    if running_from_install_dir() {
        Mode::Run
    } else {
        Mode::Offer
    }
}

pub fn install_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_default();
    PathBuf::from(base).join("Programs").join(APP_ID)
}

fn installed_exe() -> PathBuf {
    install_dir().join(format!("{APP_ID}.exe"))
}

fn icon_path() -> PathBuf {
    install_dir().join(format!("{APP_ID}.ico"))
}

fn shortcut_path() -> PathBuf {
    let base = std::env::var("APPDATA").unwrap_or_default();
    PathBuf::from(base)
        .join("Microsoft\\Windows\\Start Menu\\Programs")
        .join(format!("{APP_NAME_ASCII}.lnk"))
}

pub fn running_from_install_dir() -> bool {
    let current = PathBuf::from(autostart::own_exe_path());
    let installed = installed_exe();
    // Compare case-insensitively; Windows paths are not case sensitive and the
    // shell can hand back a differently-cased path than the one we wrote.
    current.to_string_lossy().to_lowercase() == installed.to_string_lossy().to_lowercase()
}

fn message(text: &str, caption: &str, style: MESSAGEBOX_STYLE) -> i32 {
    unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from(caption), style).0 }
}

/// Ask a running instance to shut down and wait for it to let go of its files.
///
/// Reuses the normal quit path, so it unhooks, drops the tray icon and
/// unregisters the hotkey exactly as the tray's Exit does.
fn stop_running_instance() {
    let Ok(hwnd) = (unsafe { FindWindowW(WINDOW_CLASS, PCWSTR::null()) }) else {
        return;
    };
    if hwnd.is_invalid() {
        return;
    }

    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_APP_QUIT, WPARAM(0), LPARAM(0));
    }

    for _ in 0..100 {
        if !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            // The window is gone, but the process needs another moment to
            // unmap its image before the file can be replaced or deleted.
            std::thread::sleep(std::time::Duration::from_millis(200));
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

pub fn offer(silent: bool) -> Mode {
    if silent {
        return Mode::Install { silent };
    }

    let text = format!(
        "Install {APP_NAME} {VERSION} for the current user?\n\n\
         It will be installed to:\n{}\n\n\
         It will start automatically when you sign in; you can turn that off \
         from the tray icon at any time.\n\n\
         Choose No to run it from here without installing.",
        install_dir().display()
    );

    if message(&text, APP_NAME, MB_YESNO | MB_ICONQUESTION) == IDYES.0 {
        Mode::Install { silent }
    } else {
        Mode::Run
    }
}

pub fn install(silent: bool) -> i32 {
    if let Err(error) = try_install() {
        if !silent {
            message(
                &format!("Could not install {APP_NAME}.\n\n{error}"),
                APP_NAME,
                MB_OK | MB_ICONERROR,
            );
        }
        return 1;
    }

    // Start the copy that was just installed, not the one the user ran.
    let _ = std::process::Command::new(installed_exe()).spawn();

    if !silent {
        message(
            &format!(
                "{APP_NAME} {VERSION} is installed and running.\n\n\
                 Press F11 to switch a window to Bangla. To quit, right-click \
                 the tray icon and choose Exit."
            ),
            APP_NAME,
            MB_OK | MB_ICONINFORMATION,
        );
    }
    0
}

/// Delete uninstall helpers left in `%TEMP%` by earlier runs.
///
/// Each uninstall strands exactly one of these, because the last copy cannot
/// delete the file it is executing from. Sweeping on the way past keeps that
/// from accumulating: any helper other than the current process has finished
/// its work, so its file is unlocked and can go.
fn sweep_stale_finishers() {
    let own = PathBuf::from(autostart::own_exe_path());
    let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let is_finisher = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("okkhor-uninstall-") && n.ends_with(".exe"));

        if is_finisher && path != own {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn try_install() -> std::io::Result<()> {
    // Reinstalling over a running copy would fail to overwrite the exe.
    stop_running_instance();
    sweep_stale_finishers();

    let dir = install_dir();
    std::fs::create_dir_all(&dir)?;

    let source = PathBuf::from(autostart::own_exe_path());
    let target = installed_exe();
    if source != target {
        std::fs::copy(&source, &target)?;
    }

    write_icon_file(&icon_path())?;
    create_shortcut();
    register_uninstall(&target);

    // Autostart has to name the installed copy, not the one being run from a
    // downloads folder. The tray checkbox can still undo it.
    autostart::set_enabled_for(&target.display().to_string(), true);

    Ok(())
}

pub fn uninstall(silent: bool) -> i32 {
    stop_running_instance();

    autostart::set_enabled(false);
    autostart::delete_key_tree(autostart::SETTINGS_KEY);
    autostart::delete_key_tree(UNINSTALL_KEY);
    let _ = std::fs::remove_file(shortcut_path());

    let dir = install_dir();
    let current = PathBuf::from(autostart::own_exe_path());

    // If we are running from inside the directory we are deleting, we hold a
    // lock on our own image. Hand the last step to a copy in %TEMP%.
    if current.starts_with(&dir) {
        match spawn_finisher(&dir) {
            Ok(()) => {}
            Err(error) => {
                if !silent {
                    message(
                        &format!(
                            "{APP_NAME} was unregistered, but its files could not be removed.\n\n\
                             {error}\n\nDelete this folder manually:\n{}",
                            dir.display()
                        ),
                        APP_NAME,
                        MB_OK | MB_ICONERROR,
                    );
                }
                return 1;
            }
        }
    } else {
        let _ = std::fs::remove_dir_all(&dir);
    }

    if !silent {
        message(
            &format!("{APP_NAME} has been removed."),
            APP_NAME,
            MB_OK | MB_ICONINFORMATION,
        );
    }
    0
}

/// Copy ourselves to `%TEMP%` and hand over the deletion of `dir`.
fn spawn_finisher(dir: &Path) -> std::io::Result<()> {
    let pid = std::process::id();
    let temp = std::env::temp_dir().join(format!("okkhor-uninstall-{pid}.exe"));
    std::fs::copy(autostart::own_exe_path(), &temp)?;

    std::process::Command::new(&temp)
        .arg("--finish-uninstall")
        .arg(dir)
        .arg(pid.to_string())
        .spawn()?;
    Ok(())
}

/// Runs from `%TEMP%`: wait for the uninstaller to exit, then delete the
/// install directory.
///
/// This copy cannot delete itself afterwards. The usual trick, marking the file
/// for deletion on reboot with `MoveFileExW`, writes to HKLM and so needs
/// administrator rights that a per-user install deliberately never asks for, so
/// a small file is left behind in `%TEMP%` for Windows to clean up in its own
/// time.
pub fn finish_uninstall(dir: &Path, parent: u32) -> i32 {
    // A failure to open the process means it has already gone, which is the
    // outcome we were waiting for either way.
    unsafe {
        if let Ok(handle) = OpenProcess(PROCESS_SYNCHRONIZE, false, parent) {
            WaitForSingleObject(handle, 10_000);
            let _ = CloseHandle(handle);
        }
    }

    // Older helpers from previous uninstalls are no longer running, so their
    // files can go even though this one cannot remove itself.
    sweep_stale_finishers();

    // The image needs a moment to unmap even after the process object signals.
    for _ in 0..25 {
        std::thread::sleep(std::time::Duration::from_millis(200));
        if std::fs::remove_dir_all(dir).is_ok() || !dir.exists() {
            return 0;
        }
    }
    1
}

fn register_uninstall(exe: &Path) {
    let Some(key) = Key::open(UNINSTALL_KEY, true) else {
        return;
    };

    let exe = exe.display().to_string();
    let size_kb = std::fs::metadata(&exe).map(|m| m.len() / 1024).unwrap_or(0) as u32;

    key.set(w!("DisplayName"), APP_NAME);
    key.set(w!("DisplayVersion"), VERSION);
    key.set(w!("Publisher"), PUBLISHER);
    key.set(w!("DisplayIcon"), &icon_path().display().to_string());
    key.set(w!("InstallLocation"), &install_dir().display().to_string());
    key.set(w!("UninstallString"), &format!("\"{exe}\" --uninstall"));
    key.set(
        w!("QuietUninstallString"),
        &format!("\"{exe}\" --uninstall --silent"),
    );
    // Apps & Features hides the Modify and Repair buttons when these are set.
    key.set_dword(w!("NoModify"), 1);
    key.set_dword(w!("NoRepair"), 1);
    key.set_dword(w!("EstimatedSize"), size_kb);
}

/// Create the Start Menu shortcut through the shell's own COM object, which is
/// the only supported way to author a `.lnk`.
fn create_shortcut() {
    unsafe {
        // Ignore the HRESULT: S_FALSE means this thread was already
        // initialised, which is just as good for our purposes.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);

        if let Ok(link) = CoCreateInstance::<_, IShellLinkW>(&ShellLink, None, CLSCTX_INPROC_SERVER)
        {
            let exe = HSTRING::from(installed_exe().display().to_string());
            let _ = link.SetPath(&exe);
            let _ = link.SetWorkingDirectory(&HSTRING::from(install_dir().display().to_string()));
            let _ = link.SetIconLocation(&HSTRING::from(icon_path().display().to_string()), 0);
            let _ = link.SetDescription(w!("Bangla phonetic input"));

            if let Ok(file) = link.cast::<IPersistFile>() {
                let _ = file.Save(&HSTRING::from(shortcut_path().display().to_string()), true);
            }
        }

        CoUninitialize();
    }
}

/// Write a real `.ico`, rendered from the same artwork as the tray icon.
///
/// The binary has no embedded icon resource — the tray icons are drawn with GDI
/// at run time — and embedding one would need a resource compiler or an extra
/// crate. Generating the file at install time keeps the dependency list empty
/// and still gives the Start Menu shortcut and Apps & Features a real icon.
fn write_icon_file(path: &Path) -> std::io::Result<()> {
    const SIZES: [i32; 2] = [32, 48];

    let images: Vec<(i32, Vec<u8>)> = SIZES
        .iter()
        .map(|&size| (size, ico_image(size, &crate::tray::app_icon_pixels(size))))
        .collect();

    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // 1 = icon
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());

    // Directory entries come first, so the offsets have to account for all of
    // them before any pixels are written.
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, data) in &images {
        out.push(*size as u8);
        out.push(*size as u8);
        out.push(0); // palette size: none, this is a true-colour image
        out.push(0); // reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in &images {
        out.extend_from_slice(data);
    }

    std::fs::write(path, out)
}

/// One image inside an `.ico`: a BITMAPINFOHEADER, the colour rows bottom-up,
/// then the AND mask.
fn ico_image(size: i32, top_down_bgra: &[u8]) -> Vec<u8> {
    let size = size as usize;
    let mask_stride = size.div_ceil(32) * 4;
    let mut data = Vec::with_capacity(40 + size * size * 4 + mask_stride * size);

    data.extend_from_slice(&40u32.to_le_bytes()); // biSize
    data.extend_from_slice(&(size as i32).to_le_bytes()); // biWidth
    // biHeight covers the colour rows *and* the mask rows, hence double.
    data.extend_from_slice(&(size as i32 * 2).to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes()); // biPlanes
    data.extend_from_slice(&32u16.to_le_bytes()); // biBitCount
    data.extend_from_slice(&0u32.to_le_bytes()); // biCompression = BI_RGB
    data.extend_from_slice(&0u32.to_le_bytes()); // biSizeImage
    data.extend_from_slice(&0i32.to_le_bytes()); // biXPelsPerMeter
    data.extend_from_slice(&0i32.to_le_bytes()); // biYPelsPerMeter
    data.extend_from_slice(&0u32.to_le_bytes()); // biClrUsed
    data.extend_from_slice(&0u32.to_le_bytes()); // biClrImportant

    // Icon bitmaps are stored bottom-up; our renderer produces top-down rows.
    for row in (0..size).rev() {
        let start = row * size * 4;
        data.extend_from_slice(&top_down_bgra[start..start + size * 4]);
    }

    // Fully opaque alpha makes the AND mask irrelevant, but it must be present.
    data.resize(data.len() + mask_stride * size, 0);
    data
}

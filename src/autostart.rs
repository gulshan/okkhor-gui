//! Registry-backed settings: the Windows startup entry, and the small [`Key`]
//! wrapper that [`crate::setup`] also uses to register the app in Apps &
//! Features.
//!
//! Everything here lives under `HKEY_CURRENT_USER`. The app is per-user by
//! nature — it hooks one interactive session — so nothing it writes ever needs
//! administrator rights.

use windows::Win32::Foundation::{ERROR_SUCCESS, MAX_PATH};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyW, RegDeleteTreeW, RegDeleteValueW, RegOpenKeyExW,
    RegQueryValueExW, RegSetValueExW,
};
use windows::core::{PCWSTR, w};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("okkhor-gui");

/// Anything the app stores lands under here, so uninstall can remove it in one
/// go. Nothing is written here at present; earlier versions kept per-executable
/// backspace settings, and uninstall still clears the key so those are not left
/// behind on machines that have them.
pub const SETTINGS_KEY: PCWSTR = w!("Software\\okkhor-gui");

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Reinterpret a NUL-terminated wide string's bytes as a `String`.
fn from_wide_bytes(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|&unit| unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

pub struct Key(HKEY);

impl Key {
    pub fn open(path: PCWSTR, write: bool) -> Option<Self> {
        let mut handle = HKEY::default();
        let access = if write {
            KEY_READ | KEY_WRITE
        } else {
            KEY_READ
        };
        let status = if write {
            unsafe {
                RegCreateKeyExW(
                    HKEY_CURRENT_USER,
                    path,
                    None,
                    PCWSTR::null(),
                    REG_OPTION_NON_VOLATILE,
                    access,
                    None,
                    &mut handle,
                    None,
                )
            }
        } else {
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path, None, access, &mut handle) }
        };

        (status == ERROR_SUCCESS).then_some(Key(handle))
    }

    pub fn set(&self, name: PCWSTR, value: &str) {
        let data = wide(value);
        let bytes = unsafe {
            std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(&data[..]))
        };
        unsafe {
            let _ = RegSetValueExW(self.0, name, None, REG_SZ, Some(bytes));
        };
    }

    /// Apps & Features reads `NoModify`, `NoRepair` and `EstimatedSize` as
    /// DWORDs; written as strings they are ignored.
    pub fn set_dword(&self, name: PCWSTR, value: u32) {
        unsafe {
            let _ = RegSetValueExW(self.0, name, None, REG_DWORD, Some(&value.to_le_bytes()));
        };
    }

    pub fn get(&self, name: PCWSTR) -> Option<String> {
        let mut size = 0u32;
        let status = unsafe { RegQueryValueExW(self.0, name, None, None, None, Some(&mut size)) };
        if status != ERROR_SUCCESS {
            return None;
        }
        let mut buffer = vec![0u8; size as usize];
        let status = unsafe {
            RegQueryValueExW(
                self.0,
                name,
                None,
                None,
                Some(buffer.as_mut_ptr()),
                Some(&mut size),
            )
        };
        (status == ERROR_SUCCESS).then(|| from_wide_bytes(&buffer))
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        };
    }
}

/// Delete a key under HKCU along with everything beneath it.
///
/// `RegDeleteTreeW` clears the descendants; the follow-up `RegDeleteKeyW`
/// removes the now-empty key itself, since the tree call is documented
/// ambiguously on that point and an extra failing delete costs nothing.
pub fn delete_key_tree(path: PCWSTR) {
    unsafe {
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, path);
        let _ = RegDeleteKeyW(HKEY_CURRENT_USER, path);
    }
}

/// Path of this executable, quoted so a path containing spaces survives the
/// shell that launches it at logon.
pub fn own_exe_path() -> String {
    let mut buffer = [0u16; MAX_PATH as usize];
    let written = unsafe { GetModuleFileNameW(None, &mut buffer) };
    String::from_utf16_lossy(&buffer[..written as usize])
}

pub fn is_enabled() -> bool {
    Key::open(RUN_KEY, false)
        .and_then(|key| key.get(RUN_VALUE))
        .is_some()
}

/// Toggle autostart for the currently running executable.
pub fn set_enabled(enabled: bool) {
    set_enabled_for(&own_exe_path(), enabled);
}

/// Toggle autostart for a specific executable.
///
/// Install needs this: at that moment the running image is whatever copy the
/// user launched — typically still sitting in a downloads folder — while the
/// entry has to point at the copy that was just installed. Writing
/// `own_exe_path()` there would leave autostart aimed at a file the user is
/// about to delete.
pub fn set_enabled_for(exe: &str, enabled: bool) {
    let Some(key) = Key::open(RUN_KEY, true) else {
        return;
    };
    if enabled {
        key.set(RUN_VALUE, &format!("\"{exe}\""));
    } else {
        unsafe {
            let _ = RegDeleteValueW(key.0, RUN_VALUE);
        };
    }
}

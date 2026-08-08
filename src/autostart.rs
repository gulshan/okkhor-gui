//! Registry-backed settings: the Windows startup entry and the per-application
//! erase-mode overrides.

use std::collections::HashMap;

use windows::Win32::Foundation::{ERROR_SUCCESS, MAX_PATH};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteValueW, RegEnumValueW, RegOpenKeyExW, RegQueryValueExW,
    RegSetValueExW,
};
use windows::core::{PCWSTR, PWSTR, w};

use crate::bengali::EraseMode;

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: PCWSTR = w!("okkhor-gui");
const ERASE_KEY: PCWSTR = w!("Software\\okkhor-gui\\EraseMode");

fn wide(text: &str) -> Vec<u16> {
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

struct Key(HKEY);

impl Key {
    fn open(path: PCWSTR, write: bool) -> Option<Self> {
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

    fn set(&self, name: PCWSTR, value: &str) {
        let data = wide(value);
        let bytes = unsafe {
            std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(&data[..]))
        };
        unsafe {
            let _ = RegSetValueExW(self.0, name, None, REG_SZ, Some(bytes));
        };
    }

    fn get(&self, name: PCWSTR) -> Option<String> {
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

pub fn set_enabled(enabled: bool) {
    let Some(key) = Key::open(RUN_KEY, true) else {
        return;
    };
    if enabled {
        key.set(RUN_VALUE, &format!("\"{}\"", own_exe_path()));
    } else {
        unsafe {
            let _ = RegDeleteValueW(key.0, RUN_VALUE);
        };
    }
}

/// All remembered per-executable erase modes, keyed by full image path.
pub fn load_erase_overrides() -> HashMap<String, EraseMode> {
    let mut overrides = HashMap::new();
    let Some(key) = Key::open(ERASE_KEY, false) else {
        return overrides;
    };

    for index in 0.. {
        // Value names are bounded by 16383 characters; paths are far shorter,
        // but the buffer has to satisfy the API.
        let mut name = [0u16; 512];
        let mut name_len = name.len() as u32;
        let mut data = [0u8; 64];
        let mut data_len = data.len() as u32;

        let status = unsafe {
            RegEnumValueW(
                key.0,
                index,
                Some(PWSTR(name.as_mut_ptr())),
                &mut name_len,
                None,
                None,
                Some(data.as_mut_ptr()),
                Some(&mut data_len),
            )
        };
        if status != ERROR_SUCCESS {
            break;
        }

        let path = String::from_utf16_lossy(&name[..name_len as usize]);
        let mode = EraseMode::from_registry_str(&from_wide_bytes(&data[..data_len as usize]));
        overrides.insert(path, mode);
    }

    overrides
}

pub fn save_erase_override(exe_path: &str, mode: EraseMode) {
    if let Some(key) = Key::open(ERASE_KEY, true) {
        let name = wide(exe_path);
        key.set(PCWSTR(name.as_ptr()), mode.as_registry_str());
    }
}

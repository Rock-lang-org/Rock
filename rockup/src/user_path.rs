//! The per-user environment in `HKCU\Environment` on Windows. rockup adds its
//! bin directory to the user `PATH` there instead of editing shell startup
//! files, as rustup does. Values keep their registry type, so `%VAR%`
//! references in an expandable `PATH` survive the update.

use std::{ffi::OsStr, os::windows::ffi::OsStrExt, path::Path, ptr};

use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
    System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER,
        KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_OPTION_NON_VOLATILE, REG_SZ,
    },
    UI::WindowsAndMessaging::{
        SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
    },
};

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

fn open_environment() -> Result<Key, String> {
    let mut key: HKEY = 0;
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide("Environment").as_ptr(),
            0,
            ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE,
            ptr::null(),
            &mut key,
            ptr::null_mut(),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!(
            "Failed to open HKCU\\Environment: {}",
            std::io::Error::from_raw_os_error(status as i32)
        ));
    }
    Ok(Key(key))
}

/// A string value without its terminator, and its type (`REG_SZ` or
/// `REG_EXPAND_SZ`), unexpanded.
fn read_string(key: &Key, name: &str) -> Result<Option<(Vec<u16>, u32)>, String> {
    let name = wide(name);
    let mut kind = 0;
    let mut size = 0;
    let failed = |status: u32| {
        format!(
            "Failed to read the user environment: {}",
            std::io::Error::from_raw_os_error(status as i32)
        )
    };
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            ptr::null(),
            &mut kind,
            ptr::null_mut(),
            &mut size,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if status != ERROR_SUCCESS {
        return Err(failed(status));
    }
    if kind != REG_SZ && kind != REG_EXPAND_SZ {
        return Err("The user environment value is not a string".into());
    }
    let mut data = vec![0u16; size as usize / 2 + 1];
    let mut size = (data.len() * 2) as u32;
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            ptr::null(),
            &mut kind,
            data.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(failed(status));
    }
    data.truncate(size as usize / 2);
    if let Some(end) = data.iter().position(|unit| *unit == 0) {
        data.truncate(end);
    }
    Ok(Some((data, kind)))
}

fn write_string(key: &Key, name: &str, value: &[u16], kind: u32) -> Result<(), String> {
    let mut data = value.to_vec();
    data.push(0);
    let status = unsafe {
        RegSetValueExW(
            key.0,
            wide(name).as_ptr(),
            0,
            kind,
            data.as_ptr().cast(),
            (data.len() * 2) as u32,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!(
            "Failed to update the user environment: {}",
            std::io::Error::from_raw_os_error(status as i32)
        ));
    }
    Ok(())
}

/// Tells running programs, such as Explorer, to reload the environment, so
/// new terminals see the change.
fn broadcast_change() {
    let area = wide("Environment");
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            area.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            5000,
            ptr::null_mut(),
        )
    };
}

fn same_entry(left: &[u16], right: &[u16]) -> bool {
    let normalize = |entry: &[u16]| {
        String::from_utf16_lossy(entry)
            .trim_end_matches(['\\', '/'])
            .to_lowercase()
    };
    normalize(left) == normalize(right)
}

/// Prepends `dir` to the user `PATH` unless it is already listed. Returns
/// whether the value changed.
pub(crate) fn ensure_on_user_path(dir: &Path) -> Result<bool, String> {
    let key = open_environment()?;
    let entry: Vec<u16> = dir.as_os_str().encode_wide().collect();
    let (current, kind) = read_string(&key, "Path")?.unwrap_or((Vec::new(), REG_EXPAND_SZ));
    if current
        .split(|unit| *unit == u16::from(b';'))
        .any(|existing| same_entry(existing, &entry))
    {
        return Ok(false);
    }
    let mut updated = entry;
    if !current.is_empty() {
        updated.push(u16::from(b';'));
        updated.extend(current);
    }
    write_string(&key, "Path", &updated, kind)?;
    broadcast_change();
    Ok(true)
}

/// Sets a user environment variable, such as a custom `ROCKUP_HOME`.
pub(crate) fn set_user_variable(name: &str, value: &OsStr) -> Result<(), String> {
    let key = open_environment()?;
    let value: Vec<u16> = value.encode_wide().collect();
    if read_string(&key, name)?.is_some_and(|(current, _)| current == value) {
        return Ok(());
    }
    write_string(&key, name, &value, REG_SZ)?;
    broadcast_change();
    Ok(())
}

use std::io;
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, LPARAM, WPARAM};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_OPTION_NON_VOLATILE, REG_SZ, REG_VALUE_TYPE,
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyW, RegQueryValueExW, RegSetValueExW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
};
use windows::core::{HSTRING, PCWSTR, w};

use super::place::{HOST_NAME, host_json};

const BROWSER_KEYS: &[(&str, &str)] = &[
    ("brave", r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts"),
    ("chrome", r"Software\Google\Chrome\NativeMessagingHosts"),
    ("edge", r"Software\Microsoft\Edge\NativeMessagingHosts"),
    ("chromium", r"Software\Chromium\NativeMessagingHosts"),
];
const BROADCAST_MS: u32 = 2000;

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        let _ = unsafe { RegCloseKey(self.0) };
    }
}

fn create(path: &str) -> io::Result<Key> {
    let mut key = HKEY::default();
    let path = HSTRING::from(path);
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &path,
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE,
            None,
            &raw mut key,
            None,
        )
    }
    .ok()
    .map_err(io::Error::other)?;
    Ok(Key(key))
}

fn wide(text: &str) -> Vec<u8> {
    text.encode_utf16().chain(std::iter::once(0)).flat_map(u16::to_le_bytes).collect()
}

fn set(key: &Key, name: PCWSTR, kind: REG_VALUE_TYPE, text: &str) -> io::Result<()> {
    unsafe { RegSetValueExW(key.0, name, None, kind, Some(&wide(text))) }.ok().map_err(io::Error::other)
}

fn get(key: &Key, name: PCWSTR) -> io::Result<Option<(String, REG_VALUE_TYPE)>> {
    let mut kind = REG_VALUE_TYPE::default();
    let mut size = 0u32;
    let first = unsafe { RegQueryValueExW(key.0, name, None, Some(&raw mut kind), None, Some(&raw mut size)) };
    if first == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    first.ok().map_err(io::Error::other)?;
    let mut buf = vec![0u8; size as usize];
    unsafe { RegQueryValueExW(key.0, name, None, Some(&raw mut kind), Some(buf.as_mut_ptr()), Some(&raw mut size)) }
        .ok()
        .map_err(io::Error::other)?;
    let units: Vec<u16> = buf[..size as usize].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    let text = String::from_utf16_lossy(&units).trim_end_matches('\0').to_string();
    Ok(Some((text, kind)))
}

pub fn register(home: &Path, _installed: &[&str]) -> Vec<&'static str> {
    let target = host_json(home).display().to_string();
    BROWSER_KEYS
        .iter()
        .filter(|(_, path)| {
            create(&format!(r"{path}\{HOST_NAME}")).and_then(|k| set(&k, PCWSTR::null(), REG_SZ, &target)).is_ok()
        })
        .map(|(name, _)| *name)
        .collect()
}

pub fn unregister() {
    for (_, path) in BROWSER_KEYS {
        let full = HSTRING::from(format!(r"{path}\{HOST_NAME}"));
        let _ = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, &full) };
    }
}

pub fn path_has(path: &str, dir: &Path) -> bool {
    let want = dir.display().to_string().trim_end_matches('\\').to_lowercase();
    path.split(';').any(|p| p.trim_end_matches('\\').to_lowercase() == want)
}

pub fn add_to_path(exe: &Path) -> io::Result<Option<PathBuf>> {
    let Some(dir) = exe.parent() else { return Ok(None) };
    let key = create("Environment")?;
    let (cur, kind) = get(&key, w!("Path"))?.unwrap_or((String::new(), REG_EXPAND_SZ));
    if path_has(&cur, dir) {
        return Ok(None);
    }
    let mut parts: Vec<&str> = cur.split(';').filter(|p| !p.is_empty()).collect();
    let add = dir.display().to_string();
    parts.push(&add);
    set(&key, w!("Path"), kind, &parts.join(";"))?;
    let env = w!("Environment");
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(env.as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            BROADCAST_MS,
            None,
        )
    };
    Ok(Some(dir.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_entries_match_without_case_or_trailing_slash() {
        let dir = Path::new(r"C:\Users\x\.wisply\bin");
        assert!(path_has(r"C:\Windows;c:\users\X\.wisply\bin\;D:\y", dir));
        assert!(!path_has(r"C:\Windows;C:\Users\x\.wisply\bin2", dir));
        assert_eq!(wide("A"), vec![65, 0, 0, 0]);
    }
}

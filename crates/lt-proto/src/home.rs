use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub fn user() -> String {
    ["USERNAME", "USER"]
        .iter()
        .filter_map(|k| env::var(k).ok())
        .find(|v| !v.is_empty())
        .unwrap_or_else(|| "user".into())
        .to_lowercase()
}

fn user_dir() -> PathBuf {
    PathBuf::from(env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default())
}

pub fn default_home() -> PathBuf {
    user_dir().join(".wisply")
}

pub fn old_home() -> PathBuf {
    user_dir().join(".lighting")
}

pub fn home_var() -> Option<PathBuf> {
    ["WISPLY_HOME", "LIGHTING_HOME"].iter().filter_map(env::var_os).find(|h| !h.is_empty()).map(PathBuf::from)
}

pub fn home() -> PathBuf {
    home_var().unwrap_or_else(default_home)
}

pub fn address(home: &Path) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!(r"\\.\pipe\{}", pipe_name(home)))
    } else {
        home.join(format!("wisply-{}.sock", user()))
    }
}

fn norm(path: &Path) -> String {
    let text = path.to_string_lossy().replace('/', "\\");
    let text = text.trim_end_matches('\\');
    if cfg!(windows) { text.to_lowercase() } else { text.to_string() }
}

pub fn is_default_home(home: &Path) -> bool {
    norm(home) == norm(&default_home())
}

fn pipe_name(home: &Path) -> String {
    let base = format!("wisply-{}", user());
    if is_default_home(home) {
        return base;
    }
    let hash = norm(home).bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    format!("{base}-{hash:08x}")
}

pub fn key_token(home: &Path) -> Option<String> {
    let key = fs::read(home.join("key")).ok()?;
    (key.len() >= 32).then(|| hex(&key[..32]))
}

pub fn make_key(home: &Path) -> io::Result<String> {
    if let Some(token) = key_token(home) {
        return Ok(token);
    }
    fs::create_dir_all(home)?;
    let key = random_bytes()?;
    fs::write(home.join("key"), key)?;
    Ok(hex(&key))
}

#[cfg(windows)]
fn random_bytes() -> io::Result<[u8; 32]> {
    #[link(name = "bcrypt")]
    unsafe extern "system" {
        fn BCryptGenRandom(alg: *mut std::ffi::c_void, buf: *mut u8, len: u32, flags: u32) -> i32;
    }
    const SYSTEM_RNG: u32 = 2;
    let mut key = [0u8; 32];
    let status = unsafe { BCryptGenRandom(std::ptr::null_mut(), key.as_mut_ptr(), 32, SYSTEM_RNG) };
    if status == 0 { Ok(key) } else { Err(io::Error::other(format!("no random bytes (status {status})"))) }
}

#[cfg(not(windows))]
fn random_bytes() -> io::Result<[u8; 32]> {
    use std::io::Read;
    let mut key = [0u8; 32];
    fs::File::open("/dev/urandom")?.read_exact(&mut key)?;
    Ok(key)
}

pub fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    bytes.iter().flat_map(|&b| [DIGITS[usize::from(b >> 4)] as char, DIGITS[usize::from(b & 15)] as char]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_is_the_hex_of_the_first_32_key_bytes() {
        let dir = env::temp_dir().join(format!("lt-proto-key-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let key: Vec<u8> = (0u8..40).collect();
        fs::write(dir.join("key"), &key).unwrap();
        let want: String = (0u8..32).map(|b| format!("{b:02x}")).collect();
        assert_eq!(key_token(&dir).as_deref(), Some(want.as_str()));
        fs::write(dir.join("key"), [1u8; 31]).unwrap();
        assert_eq!(key_token(&dir), None);
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(key_token(&dir), None);
    }

    #[test]
    fn a_new_home_gets_a_key_once_and_its_own_address() {
        let dir = env::temp_dir().join(format!("lt-proto-make-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let token = make_key(&dir).unwrap();
        assert_eq!(token.len(), 64);
        assert_eq!(make_key(&dir).unwrap(), token);
        assert_eq!(key_token(&dir).as_deref(), Some(token.as_str()));
        assert_ne!(address(&dir), address(&default_home()));
        assert_eq!(address(&dir), address(&dir));
        assert!(!is_default_home(&dir) && is_default_home(&default_home()));
        let spelled = default_home().to_string_lossy().to_uppercase().replace('\\', "/") + "/";
        assert_eq!(is_default_home(Path::new(&spelled)), cfg!(windows));
        if cfg!(windows) {
            let name = format!(r"\\.\pipe\wisply-{}", user());
            assert_eq!(address(&default_home()), PathBuf::from(name));
        }
        assert!(same_token(&token, &token) && !same_token(&token, "ab") && !same_token("ab", "ac"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hex_and_user_look_right() {
        assert_eq!(hex(&[0, 171, 255]), "00abff");
        let name = user();
        assert!(!name.is_empty());
        assert_eq!(name, name.to_lowercase());
    }
}

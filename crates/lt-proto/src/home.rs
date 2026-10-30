use std::env;
use std::fs;
use std::path::{Path, PathBuf};

pub fn user() -> String {
    ["USERNAME", "USER"]
        .iter()
        .filter_map(|k| env::var(k).ok())
        .find(|v| !v.is_empty())
        .unwrap_or_else(|| "user".into())
        .to_lowercase()
}

pub fn home() -> PathBuf {
    match env::var_os("LIGHTING_HOME") {
        Some(h) if !h.is_empty() => PathBuf::from(h),
        _ => PathBuf::from(env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).unwrap_or_default())
            .join(".lighting"),
    }
}

pub fn key_token(home: &Path) -> Option<String> {
    let key = fs::read(home.join("key")).ok()?;
    (key.len() >= 32).then(|| hex(&key[..32]))
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
    fn hex_and_user_look_right() {
        assert_eq!(hex(&[0, 171, 255]), "00abff");
        let name = user();
        assert!(!name.is_empty());
        assert_eq!(name, name.to_lowercase());
    }
}

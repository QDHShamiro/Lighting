use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use base64::Engine;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const HOST_NAME: &str = "dev.qdhshamiro.lighting";
pub const EXES: &[&str] = &["lighting", "lighting-ui", "lighting-task"];
const SOURCE_DEPTH: usize = 4;
const BLOCKLIST: &str = "paypal.com\npaypal.me\nstripe.com\ncheckout.stripe.com\nklarna.com\nsofort.com\ngiropay.de\n\
paysafecard.com\nwise.com\nrevolut.com\nn26.com\nbunq.com\nsparkasse.de\ndkb.de\ning.de\ncomdirect.de\ncommerzbank.de\n\
deutsche-bank.de\npostbank.de\nconsorsbank.de\nvolksbank.de\nvr.de\ntargobank.de\nsantander.de\nhypovereinsbank.de\n\
bankofamerica.com\nchase.com\nwellsfargo.com\n";

pub fn exe_name(stem: &str) -> String {
    if cfg!(windows) { format!("{stem}.exe") } else { stem.to_string() }
}

pub fn source_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors()
        .skip(1)
        .take(SOURCE_DEPTH)
        .find(|d| d.join("extension").join("manifest.json").is_file())
        .map(Path::to_path_buf)
}

pub fn extension_id(manifest: &Value) -> Option<String> {
    let key = manifest["key"].as_str()?;
    let raw = base64::engine::general_purpose::STANDARD.decode(key.trim()).ok()?;
    let digest = Sha256::digest(&raw);
    Some(digest.iter().take(16).flat_map(|b| [b >> 4, b & 15]).map(|n| char::from(b'a' + n)).collect())
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

pub fn write_if_changed(path: &Path, data: &[u8]) -> io::Result<bool> {
    if fs::read(path).is_ok_and(|old| old == data) {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, data)?;
    Ok(true)
}

fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &to)?;
        } else {
            write_if_changed(&to, &fs::read(entry.path())?)?;
        }
    }
    Ok(())
}

pub fn copy_extension(home: &Path, root: Option<&Path>) -> io::Result<Option<String>> {
    let Some(src) = root.map(|r| r.join("extension")).filter(|s| s.join("manifest.json").is_file()) else {
        return Ok(read_json(&home.join("extension").join("manifest.json")).as_ref().and_then(extension_id));
    };
    let dst = home.join("extension");
    copy_tree(&src, &dst)?;
    let mut manifest = read_json(&dst.join("manifest.json")).unwrap_or_else(|| json!({}));
    manifest["version"] = json!(lt_core::ext_version(env!("CARGO_PKG_VERSION")));
    let text = serde_json::to_string_pretty(&manifest).map_err(io::Error::other)?;
    write_if_changed(&dst.join("manifest.json"), text.as_bytes())?;
    Ok(extension_id(&manifest))
}

pub fn copy_skill(root: Option<&Path>, dst: &Path) -> io::Result<()> {
    match root.map(|r| r.join("skills").join("lighting")).filter(|s| s.is_dir()) {
        Some(src) => copy_tree(&src, dst),
        None => Err(io::Error::other("the skill files are missing next to lighting")),
    }
}

pub fn blocklist(home: &Path) -> io::Result<()> {
    let path = home.join("blocklist.txt");
    if path.exists() { Ok(()) } else { fs::write(path, BLOCKLIST) }
}

pub fn copy_exes(home: &Path) -> io::Result<PathBuf> {
    let bin = home.join("bin");
    let me = std::env::current_exe()?;
    let dst = bin.join(exe_name("lighting"));
    let from = me.parent().map(Path::to_path_buf).unwrap_or_default();
    if same_file(&from, &bin) {
        return Ok(dst);
    }
    fs::create_dir_all(&bin)?;
    for stem in EXES {
        let src = from.join(exe_name(stem));
        if src.is_file() {
            replace(&src, &bin.join(exe_name(stem)))?;
        }
    }
    Ok(dst)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

fn replace(src: &Path, dst: &Path) -> io::Result<()> {
    let data = fs::read(src)?;
    if fs::read(dst).is_ok_and(|old| old == data) {
        return Ok(());
    }
    let old = dst.with_extension("old");
    let _ = fs::remove_file(&old);
    let _ = fs::rename(dst, &old);
    fs::write(dst, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dst, fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

pub fn host_json(home: &Path) -> PathBuf {
    home.join(format!("{HOST_NAME}.json"))
}

pub fn write_host(home: &Path, exe: &Path, ext_id: Option<&str>) -> io::Result<()> {
    let origins: Vec<String> = ext_id.map(|id| format!("chrome-extension://{id}/")).into_iter().collect();
    let manifest = json!({
        "name": HOST_NAME,
        "description": "Lighting native host",
        "path": exe.display().to_string(),
        "type": "stdio",
        "allowed_origins": origins,
    });
    let text = serde_json::to_string_pretty(&manifest).map_err(io::Error::other)?;
    write_if_changed(&host_json(home), text.as_bytes()).map(|_| ())
}

#[cfg(not(windows))]
pub fn browser_dirs() -> Vec<(&'static str, PathBuf)> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    if cfg!(target_os = "macos") {
        let s = home.join("Library").join("Application Support");
        vec![
            ("brave", s.join("BraveSoftware").join("Brave-Browser")),
            ("chrome", s.join("Google").join("Chrome")),
            ("edge", s.join("Microsoft Edge")),
            ("chromium", s.join("Chromium")),
        ]
    } else {
        let c = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"));
        vec![
            ("brave", c.join("BraveSoftware").join("Brave-Browser")),
            ("chrome", c.join("google-chrome")),
            ("edge", c.join("microsoft-edge")),
            ("chromium", c.join("chromium")),
        ]
    }
}

#[cfg(not(windows))]
pub fn register(home: &Path, installed: &[&str]) -> Vec<&'static str> {
    let data = fs::read(host_json(home)).unwrap_or_default();
    browser_dirs()
        .into_iter()
        .filter(|(name, dir)| dir.exists() || installed.contains(name))
        .filter(|(_, dir)| {
            write_if_changed(&dir.join("NativeMessagingHosts").join(format!("{HOST_NAME}.json")), &data).is_ok()
        })
        .map(|(name, _)| name)
        .collect()
}

#[cfg(not(windows))]
pub fn unregister() {
    for (_, dir) in browser_dirs() {
        let _ = fs::remove_file(dir.join("NativeMessagingHosts").join(format!("{HOST_NAME}.json")));
    }
}

#[cfg(windows)]
pub use super::reg::{add_to_path, register, unregister};

#[cfg(not(windows))]
pub fn add_to_path(exe: &Path) -> io::Result<Option<PathBuf>> {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let link = home.join(".local").join("bin").join("lighting");
    if fs::read_link(&link).is_ok_and(|to| to == exe) || (link.exists() && !link.is_symlink()) {
        return Ok(None);
    }
    if let Some(dir) = link.parent() {
        fs::create_dir_all(dir)?;
    }
    if link.is_symlink() {
        fs::remove_file(&link)?;
    }
    std::os::unix::fs::symlink(exe, &link)?;
    Ok(Some(link))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extension_id_comes_from_the_manifest_key_like_chrome() {
        let manifest: Value =
            serde_json::from_str(&fs::read_to_string("../../extension/manifest.json").unwrap()).unwrap();
        assert_eq!(extension_id(&manifest).as_deref(), Some("gndlnbibcgfnkhgfipjpnpaadkghhgeg"));
        assert_eq!(extension_id(&json!({})), None);
    }

    #[test]
    fn copies_skip_unchanged_files_and_the_host_points_at_the_exe() {
        let dir = std::env::temp_dir().join(format!("lt-place-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        assert!(write_if_changed(&dir.join("a/b.txt"), b"x").unwrap());
        assert!(!write_if_changed(&dir.join("a/b.txt"), b"x").unwrap());
        write_host(&dir, &dir.join("bin").join(exe_name("lighting")), Some("abc")).unwrap();
        let host = read_json(&host_json(&dir)).unwrap();
        assert_eq!(host["allowed_origins"], json!(["chrome-extension://abc/"]));
        assert!(host["path"].as_str().unwrap().ends_with(&exe_name("lighting")));
        blocklist(&dir).unwrap();
        assert!(fs::read_to_string(dir.join("blocklist.txt")).unwrap().contains("paypal.com"));
        let _ = fs::remove_dir_all(&dir);
    }
}

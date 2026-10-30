use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use ureq::tls::{RootCerts, TlsConfig};

const RECORD: &str = "tools.json";
const API: &str = "https://api.github.com/repos/";
const YTDLP_REPO: &str = "yt-dlp/yt-dlp";
const YTDLP_SUMS: &str = "SHA2-256SUMS";
const FFMPEG_REPO: &str = "BtbN/FFmpeg-Builds";
const FFMPEG_SUMS: &str = "checksums.sha256";
const DOWNLOAD_WAIT: Duration = Duration::from_secs(900);
const DOWNLOAD_MAX: u64 = 400_000_000;

static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .timeout_global(Some(DOWNLOAD_WAIT))
        .http_status_as_error(false)
        .tls_config(TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build())
        .build()
        .into()
});

pub struct Tool {
    pub name: &'static str,
    pub files: &'static [&'static str],
}

pub const TOOLS: [Tool; 2] =
    [Tool { name: "yt-dlp", files: &["yt-dlp"] }, Tool { name: "ffmpeg", files: &["ffmpeg", "ffprobe"] }];

pub fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

fn record(home: &Path) -> Map<String, Value> {
    fs::read_to_string(home.join("bin").join(RECORD))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

pub fn sha256_file(path: &Path) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    Some(hex(&hash.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub enum Check {
    Missing,
    Verified(String),
    Changed(String),
}

pub fn verify(home: &Path, tool: &Tool) -> Check {
    let all = record(home);
    let Some(entry) = all.get(tool.name) else { return Check::Missing };
    let version = entry["version"].as_str().unwrap_or("?").to_string();
    for file in tool.files {
        let path = home.join("bin").join(exe(file));
        let want = entry["sha256"][*file].as_str().unwrap_or("");
        match sha256_file(&path) {
            Some(got) if got == want => {}
            Some(_) => return Check::Changed(format!("{} differs from what doctor installed", exe(file))),
            None => return Check::Changed(format!("{} is gone", exe(file))),
        }
    }
    Check::Verified(version)
}

fn api(path: &str) -> Result<Value, String> {
    let mut resp = AGENT
        .get(format!("{API}{path}"))
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", concat!("lighting/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(|e| format!("GitHub not reachable ({e})"))?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().with_config().limit(10_000_000).read_to_string().unwrap_or_default();
    if status != 200 {
        return Err(format!("GitHub answered {status}"));
    }
    serde_json::from_str(&body).map_err(|_| "GitHub sent no JSON".into())
}

fn asset_url(release: &Value, name: &str) -> Result<String, String> {
    release["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|a| a["name"].as_str() == Some(name))
        .and_then(|a| a["browser_download_url"].as_str().map(String::from))
        .ok_or_else(|| format!("the release has no {name}"))
}

fn text(url: &str) -> Result<String, String> {
    let mut resp = AGENT.get(url).call().map_err(|e| format!("download failed ({e})"))?;
    if resp.status().as_u16() != 200 {
        return Err(format!("download answered {}", resp.status().as_u16()));
    }
    resp.body_mut().with_config().limit(1_000_000).read_to_string().map_err(|e| e.to_string())
}

pub fn sum_for(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut parts = l.split_whitespace();
        let hash = parts.next()?;
        let file = parts.next()?.trim_start_matches('*');
        (file == name && hash.len() == 64).then(|| hash.to_lowercase())
    })
}

fn download(url: &str, to: &Path, want: &str) -> Result<(), String> {
    let mut resp = AGENT.get(url).call().map_err(|e| format!("download failed ({e})"))?;
    if resp.status().as_u16() != 200 {
        return Err(format!("download answered {}", resp.status().as_u16()));
    }
    let mut reader = resp.body_mut().with_config().limit(DOWNLOAD_MAX).reader();
    let mut out = File::create(to).map_err(|e| format!("cannot write {} ({e})", to.display()))?;
    let mut hash = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("download broke off ({e})"))?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(|e| format!("cannot write {} ({e})", to.display()))?;
    }
    drop(out);
    let got = hex(&hash.finalize());
    if got != want {
        let _ = fs::remove_file(to);
        return Err(format!(
            "SHA-256 does not match the release list (got {}..., want {}...)",
            &got[..12],
            &want[..12]
        ));
    }
    Ok(())
}

fn executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o755));
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn save(home: &Path, name: &str, version: &str, files: &[&str]) {
    let mut all = record(home);
    let bin = home.join("bin");
    let sums: Map<String, Value> =
        files.iter().map(|f| ((*f).to_string(), json!(sha256_file(&bin.join(exe(f))).unwrap_or_default()))).collect();
    let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    all.insert(name.into(), json!({"version": version, "sha256": sums, "at": at}));
    let _ = fs::write(bin.join(RECORD), Value::Object(all).to_string());
}

fn installed(home: &Path, name: &str) -> Option<String> {
    record(home).get(name).and_then(|e| e["version"].as_str().map(String::from))
}

fn ytdlp_asset() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", _) => Some("yt-dlp.exe"),
        ("macos", _) => Some("yt-dlp_macos"),
        ("linux", "x86_64") => Some("yt-dlp_linux"),
        ("linux", "aarch64") => Some("yt-dlp_linux_aarch64"),
        _ => None,
    }
}

fn ffmpeg_asset() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("ffmpeg-master-latest-win64-gpl.zip"),
        ("linux", "x86_64") => Some("ffmpeg-master-latest-linux64-gpl.tar.xz"),
        ("linux", "aarch64") => Some("ffmpeg-master-latest-linuxarm64-gpl.tar.xz"),
        _ => None,
    }
}

pub fn ytdlp(home: &Path) -> Result<String, String> {
    let asset = ytdlp_asset().ok_or("no yt-dlp build for this system, install it with pip install yt-dlp")?;
    let release = api(&format!("{YTDLP_REPO}/releases/latest"))?;
    let tag = release["tag_name"].as_str().unwrap_or("?").to_string();
    if installed(home, "yt-dlp").as_deref() == Some(tag.as_str())
        && matches!(verify(home, &TOOLS[0]), Check::Verified(_))
    {
        return Ok(format!("yt-dlp {tag} is current"));
    }
    let sums = text(&asset_url(&release, YTDLP_SUMS)?)?;
    let want = sum_for(&sums, asset).ok_or_else(|| format!("{YTDLP_SUMS} lists no {asset}"))?;
    let bin = home.join("bin");
    fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    let part = bin.join(format!("{}.part", exe("yt-dlp")));
    download(&asset_url(&release, asset)?, &part, &want)?;
    let target = bin.join(exe("yt-dlp"));
    fs::rename(&part, &target).map_err(|e| format!("cannot replace {} ({e}); is it running?", target.display()))?;
    executable(&target);
    save(home, "yt-dlp", &tag, TOOLS[0].files);
    Ok(format!("yt-dlp {tag} installed in {} (SHA-256 checked)", bin.display()))
}

fn find(dir: &Path, name: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find(&path, name) {
                return Some(found);
            }
        } else if entry.file_name().to_string_lossy() == name {
            return Some(path);
        }
    }
    None
}

pub fn ffmpeg(home: &Path) -> Result<String, String> {
    let asset = ffmpeg_asset().ok_or(if cfg!(target_os = "macos") {
        "install ffmpeg with brew install ffmpeg"
    } else {
        "no ffmpeg build for this system, install it with the package manager"
    })?;
    let release = api(&format!("{FFMPEG_REPO}/releases/latest"))?;
    let version = release["published_at"].as_str().unwrap_or("?").chars().take(10).collect::<String>();
    let version = format!("build {version}");
    if installed(home, "ffmpeg").as_deref() == Some(version.as_str())
        && matches!(verify(home, &TOOLS[1]), Check::Verified(_))
    {
        return Ok(format!("ffmpeg {version} is current"));
    }
    let sums = text(&asset_url(&release, FFMPEG_SUMS)?)?;
    let want = sum_for(&sums, asset).ok_or_else(|| format!("{FFMPEG_SUMS} lists no {asset}"))?;
    let bin = home.join("bin");
    let work = home.join("bin").join("ffmpeg-download");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let archive = work.join(asset);
    let done = (|| {
        download(&asset_url(&release, asset)?, &archive, &want)?;
        let mut cmd = Command::new("tar");
        cmd.arg(if asset.ends_with(".xz") { "-xJf" } else { "-xf" })
            .arg(&archive)
            .arg("-C")
            .arg(&work)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        crate::v2::hide(&mut cmd);
        let out = cmd.output().map_err(|e| format!("cannot run tar ({e})"))?;
        if !out.status.success() {
            return Err(format!("tar could not unpack {asset}: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        for file in TOOLS[1].files {
            let found = find(&work, &exe(file)).ok_or_else(|| format!("{asset} has no {}", exe(file)))?;
            let target = bin.join(exe(file));
            fs::rename(&found, &target)
                .or_else(|_| fs::copy(&found, &target).map(|_| ()))
                .map_err(|e| format!("cannot replace {} ({e}); is it running?", target.display()))?;
            executable(&target);
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&work);
    done?;
    save(home, "ffmpeg", &version, TOOLS[1].files);
    Ok(format!("ffmpeg {version} installed in {} (SHA-256 checked)", bin.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_lists_give_the_hash_of_one_file() {
        let sums = "aa  other.exe\n0123456789abcdef0123456789abcdef0123456789abcdef0123456789ABCDEF *yt-dlp.exe\n";
        assert_eq!(
            sum_for(sums, "yt-dlp.exe").as_deref(),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );
        assert_eq!(sum_for(sums, "other.exe"), None);
        assert_eq!(sum_for(sums, "missing"), None);
        let home = std::env::temp_dir().join(format!("lt-install-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join("bin")).unwrap();
        assert!(matches!(verify(&home, &TOOLS[0]), Check::Missing));
        fs::write(home.join("bin").join(exe("yt-dlp")), b"tool").unwrap();
        save(&home, "yt-dlp", "2026.10.01", TOOLS[0].files);
        assert!(matches!(verify(&home, &TOOLS[0]), Check::Verified(v) if v == "2026.10.01"));
        fs::write(home.join("bin").join(exe("yt-dlp")), b"changed").unwrap();
        assert!(matches!(verify(&home, &TOOLS[0]), Check::Changed(_)));
        let _ = fs::remove_dir_all(&home);
    }

    #[test]
    #[ignore = "downloads about 220 MB from GitHub"]
    fn real_downloads_are_checked_and_recorded() {
        let home = std::env::temp_dir().join(format!("lt-install-real-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        let got = ytdlp(&home).unwrap();
        assert!(got.contains("installed") && got.contains("SHA-256 checked"), "{got}");
        assert!(matches!(verify(&home, &TOOLS[0]), Check::Verified(_)));
        assert!(ytdlp(&home).unwrap().ends_with("is current"));
        if ffmpeg_asset().is_some() {
            let got = ffmpeg(&home).unwrap();
            assert!(got.contains("installed"), "{got}");
            assert!(matches!(verify(&home, &TOOLS[1]), Check::Verified(_)));
            assert!(!home.join("bin").join("ffmpeg-download").exists());
        }
        let _ = fs::remove_dir_all(&home);
    }
}

use std::fs;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};

use super::place::{self, ALIAS, exe_name};

const SKIP: &[&str] = &[
    "venv",
    "bin",
    "out",
    "watch",
    "extension",
    "daemon.pid",
    "daemon.log",
    "client-root",
    "host.bat",
    "host.py",
    "run.pyw",
    "dev.qdhshamiro.lighting.json",
];
const MB: u64 = 1 << 20;

fn size(path: &Path) -> u64 {
    match fs::metadata(path) {
        Ok(m) if m.is_dir() => fs::read_dir(path).map_or(0, |d| d.flatten().map(|e| size(&e.path())).sum()),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

fn copy_all(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_all(&entry.path(), &to)?;
        } else if !to.exists() {
            fs::copy(entry.path(), &to)?;
        }
    }
    Ok(())
}

fn stop_old(exe: &Path, old: &Path) {
    if !exe.is_file() {
        return;
    }
    let _ = Command::new(exe)
        .arg("stop")
        .env_remove("WISPLY_HOME")
        .env("LIGHTING_HOME", old)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn copy_old(home: &Path, old: &Path) -> io::Result<u64> {
    let mut moved = 0;
    for entry in fs::read_dir(old)?.flatten() {
        let name = entry.file_name();
        if SKIP.iter().any(|s| name.eq_ignore_ascii_case(s)) {
            continue;
        }
        let to = home.join(&name);
        if entry.file_type()?.is_dir() {
            copy_all(&entry.path(), &to)?;
        } else if !to.exists() {
            fs::create_dir_all(home)?;
            fs::copy(entry.path(), &to)?;
        }
        moved += size(&to);
    }
    Ok(moved)
}

pub fn from_old(home: &Path, old: &Path) -> io::Result<Option<String>> {
    if !wisply_proto::is_default_home(home) || home.join("memory").exists() || !old.join("memory").is_dir() {
        return Ok(None);
    }
    let bin = old.join("bin").join(exe_name(ALIAS));
    let me = std::env::current_exe()?;
    let kept = bin.with_extension("old");
    if bin.is_file() && place::replace(&me, &bin)? {
        stop_old(&kept, old);
    } else {
        stop_old(&bin, old);
    }
    let moved = copy_old(home, old)?;
    Ok(Some(format!(
        "copied your data from {} to {} ({} MB); the old folder stays until 3.1",
        old.display(),
        home.display(),
        moved.div_ceil(MB)
    )))
}

pub fn forward_old(home: &Path, old: &Path, exe: &Path) -> io::Result<bool> {
    let bin = old.join("bin");
    if !wisply_proto::is_default_home(home) || !bin.is_dir() {
        return Ok(false);
    }
    place::replace(exe, &bin.join(exe_name(ALIAS)))
}

pub fn link_extension(home: &Path, old: &Path) -> io::Result<bool> {
    let from = old.join("extension");
    let to = home.join("extension");
    if !wisply_proto::is_default_home(home) || !to.is_dir() || !from.is_dir() || fs::read_link(&from).is_ok() {
        return Ok(false);
    }
    let aside = old.join("extension.old");
    let _ = fs::remove_dir_all(&aside);
    fs::rename(&from, &aside)?;
    #[cfg(windows)]
    let made = Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&from)
        .arg(&to)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(&to, &from).is_ok();
    if !made {
        fs::rename(&aside, &from)?;
    }
    Ok(made)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_data_is_copied_without_programs_and_caches_and_never_overwrites() {
        let dir = std::env::temp_dir().join(format!("lt-migrate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let old = dir.join("old");
        let home = dir.join("new");
        for f in ["memory/memory.db", "code/a.db", "key", "venv/x", "bin/lighting.exe", "watch/v.mp4", "daemon.pid"] {
            let p = old.join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, f).unwrap();
        }
        assert_eq!(from_old(&home, &old).unwrap(), None);
        let copied = |home: &Path| {
            ["memory/memory.db", "code/a.db", "key"].iter().all(|f| home.join(f).is_file())
                && ["venv", "bin", "watch", "daemon.pid"].iter().all(|f| !home.join(f).exists())
        };
        assert!(copy_old(&home, &old).unwrap() > 0);
        assert!(copied(&home));
        fs::write(home.join("key"), "mine").unwrap();
        copy_old(&home, &old).unwrap();
        assert_eq!(fs::read_to_string(home.join("key")).unwrap(), "mine");
        let _ = fs::remove_dir_all(&dir);
    }
}

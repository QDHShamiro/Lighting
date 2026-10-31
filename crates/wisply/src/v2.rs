use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn client_exe() -> Option<PathBuf> {
    let root = fs::read_to_string(wisply_proto::home().join("client-root")).ok()?;
    let exe = PathBuf::from(root.trim()).join("bin").join(client_name());
    let me = env::current_exe().ok().and_then(|p| p.canonicalize().ok());
    let same = me.is_some() && exe.canonicalize().ok() == me;
    (exe.is_file() && !same).then_some(exe)
}

pub fn hide(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

fn client_name() -> &'static str {
    if cfg!(windows) {
        "lighting.exe"
    } else if cfg!(target_os = "macos") {
        "lighting-macos"
    } else if cfg!(target_arch = "aarch64") {
        "lighting-linux-arm64"
    } else {
        "lighting-linux-x64"
    }
}

#![cfg_attr(windows, windows_subsystem = "windows")]

use std::env;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

#[cfg(windows)]
const NO_WINDOW: u32 = 0x0800_0000;
const CLIENT: &str = if cfg!(windows) { "wisply.exe" } else { "wisply" };

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let Some(home) = args.next().map(PathBuf::from) else { return ExitCode::from(2) };
    let Ok(exe) = env::current_exe().map(|p| p.with_file_name(CLIENT)) else { return ExitCode::from(2) };
    let mut cmd = Command::new(exe);
    cmd.args(args).env("WISPLY_HOME", &home).stdin(Stdio::null());
    if let Ok(log) = OpenOptions::new().create(true).append(true).open(home.join("schedule.log"))
        && let Ok(err) = log.try_clone()
    {
        cmd.stdout(log).stderr(err);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(NO_WINDOW);
    }
    match cmd.status() {
        Ok(s) => ExitCode::from(u8::try_from(s.code().unwrap_or(1)).unwrap_or(1)),
        Err(_) => ExitCode::from(1),
    }
}

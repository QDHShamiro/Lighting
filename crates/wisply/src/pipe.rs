use std::env;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use wisply_squeeze::{decode, read_limited};

use crate::daemon::{SQUEEZE_MAX_BYTES, SQUEEZE_USAGE};

fn which(name: &str) -> PathBuf {
    let plain = PathBuf::from(name);
    if plain.components().count() > 1 || plain.extension().is_some() || !cfg!(windows) {
        return plain;
    }
    let exts = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    env::var_os("PATH")
        .map(|p| env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .flat_map(|dir| exts.split(';').filter(|e| !e.is_empty()).map(move |e| dir.join(format!("{name}{e}"))))
        .find(|p| p.is_file())
        .unwrap_or(plain)
}

fn run_command(cmd: &[String]) -> (i32, String) {
    let started = io::pipe().and_then(|(reader, writer)| {
        let mut command = Command::new(which(&cmd[0]));
        command.args(&cmd[1..]).stdin(Stdio::null()).stdout(writer.try_clone()?).stderr(writer);
        let child = command.spawn()?;
        drop(command);
        Ok((reader, child))
    });
    match started {
        Ok((reader, mut child)) => {
            let data = read_limited(reader, SQUEEZE_MAX_BYTES);
            let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(1);
            (code, decode(&data))
        }
        Err(e) => (127, format!("cannot start {}: {e}", cmd[0])),
    }
}

fn stem(cmd: &str) -> String {
    Path::new(cmd).file_stem().map_or_else(|| cmd.to_string(), |s| s.to_string_lossy().into_owned())
}

pub fn squeeze(home: &Path, args: &[String]) -> ExitCode {
    let mut rest = args.to_vec();
    let (mut code, mut text) = (None, None);
    if let Some(at) = rest.iter().position(|a| a == "--") {
        let cmd = rest.split_off(at)[1..].to_vec();
        if cmd.is_empty() {
            println!("err: which command? -> try: {SQUEEZE_USAGE}");
            return ExitCode::from(2);
        }
        let (c, t) = run_command(&cmd);
        (code, text) = (Some(c), Some(t));
        rest.push(format!("--name={}", stem(&cmd[0])));
    } else if rest[1..].iter().all(|a| a.starts_with('-')) && !io::stdin().is_terminal() {
        text = Some(decode(&read_limited(io::stdin().lock(), SQUEEZE_MAX_BYTES)));
    }
    let Some(text) = text else { return crate::client::send(home, &rest) };
    if !rest.iter().any(|a| a == "--name" || a.starts_with("--name=")) {
        rest.push("--name=output".into());
    }
    let dir = home.join("out");
    let file = wisply_core::out_file(&dir, "squeeze-in", "txt");
    if fs::create_dir_all(&dir).and_then(|()| fs::write(&file, &text)).is_err() {
        println!("err: cannot write {} -> try: {SQUEEZE_USAGE}", file.display());
        return ExitCode::from(1);
    }
    rest.insert(1, file.to_string_lossy().into_owned());
    let exit = crate::client::send(home, &rest);
    let _ = fs::remove_file(&file);
    code.map_or(exit, |c| ExitCode::from(u8::try_from(c).unwrap_or(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_run_with_both_streams_in_one_text() {
        let cmd: Vec<String> = if cfg!(windows) {
            ["cmd", "/c", "echo out& echo err 1>&2& exit 3"].map(String::from).to_vec()
        } else {
            ["sh", "-c", "echo out; echo err 1>&2; exit 3"].map(String::from).to_vec()
        };
        let (code, text) = run_command(&cmd);
        assert_eq!(code, 3);
        assert!(text.contains("out") && text.contains("err"), "{text}");
        let (code, text) = run_command(&["no-such-tool-zz9".to_string()]);
        assert!(code == 127 && text.starts_with("cannot start no-such-tool-zz9"), "{text}");
        assert_eq!(stem("C:/x/cargo.exe"), "cargo");
    }
}

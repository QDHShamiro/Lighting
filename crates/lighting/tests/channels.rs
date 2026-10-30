use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;

const EXE: &str = env!("CARGO_BIN_EXE_lighting");

struct Home(PathBuf);

impl Home {
    fn run(&self, args: &[&str]) -> (i32, String) {
        let out = Command::new(EXE)
            .args(args)
            .env("LIGHTING_HOME", &self.0)
            .env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }, &self.0)
            .env("CLAUDE_CODE_SESSION_ID", "it-channels")
            .env_remove("MSYSTEM")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).trim_end().to_string())
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = self.run(&["stop"]);
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn site() -> u16 {
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = server.local_addr().unwrap().port();
    thread::spawn(move || {
        for link in server.incoming().flatten() {
            let mut reader = BufReader::new(link.try_clone().unwrap());
            let mut first = String::new();
            let _ = reader.read_line(&mut first);
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
                line.clear();
            }
            let (status, body) = if first.contains("/gate") {
                ("200 OK", "<html><head><title>Just a moment...</title></head><body>checking</body></html>")
            } else {
                ("403 Forbidden", "<html><body>no robots</body></html>")
            };
            let reply = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let mut link = link;
            let _ = link.write_all(reply.as_bytes());
        }
    });
    port
}

#[test]
fn channels_list_the_ways_and_remember_what_failed_where() {
    let base = if cfg!(windows) { std::env::temp_dir() } else { PathBuf::from("/tmp") };
    let path = base.join(format!("ltd-channels-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("v3"), "").unwrap();
    let home = Home(path);
    let (code, out) = home.run(&["channels"]);
    assert!(code == 0 && out.starts_with("channels (ways in the order Lighting tries them"), "{out}");
    for row in ["\ngithub   ", "\nweb      direct > ", "\nvideo    ", "\naudio    "] {
        assert!(out.contains(row), "{row:?} missing:\n{out}");
    }
    assert!(!out.contains("last:"), "{out}");
    let port = site();
    let (code, out) = home.run(&["read", &format!("http://127.0.0.1:{port}/blocked")]);
    assert!(code == 1 && out.contains("403"), "{out}");
    let (code, out) = home.run(&["read", &format!("http://127.0.0.1:{port}/gate")]);
    assert!(code == 0 && out.contains("checking"), "{out}");
    let (_, out) = home.run(&["channels"]);
    assert!(out.contains("last: direct failed ") && out.contains(" ago: the site showed a bot check"), "{out}");
    let (code, out) = home.run(&["channels", "x"]);
    assert!(code == 1 && out.starts_with("err: channels takes no argument"), "{out}");
}

#[test]
fn doctor_checks_the_parts_and_takes_only_fix() {
    let base = if cfg!(windows) { std::env::temp_dir() } else { PathBuf::from("/tmp") };
    let path = base.join(format!("ltd-doctor-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("v3"), "").unwrap();
    fs::write(path.join("config.json"), "{broken").unwrap();
    let home = Home(path);
    let (code, out) = home.run(&["doctor"]);
    assert!(out.starts_with("lighting doctor "), "{out}");
    for part in ["\nok   daemon  pid ", "\nok   gh      ", "\nFAIL data    ", "config.json"] {
        assert!(out.contains(part) || (part.contains("gh") && out.contains(" gh ")), "{part:?} missing:\n{out}");
    }
    assert!(code == 1 && out.lines().last().unwrap_or("").starts_with("1 problem, "), "{out}");
    let (code, _) = home.run(&["doctor", "--bogus"]);
    assert_eq!(code, 2);
}

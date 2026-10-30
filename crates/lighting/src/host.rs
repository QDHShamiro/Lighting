use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{self, Command, Stdio};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crate::v2;
use lt_proto::{Conn, Link};
use serde_json::{Value, json};

const MAX_IN: usize = 64 * 1024 * 1024;
const BOOT_WAIT: Duration = Duration::from_secs(10);
const BUSY_WAIT: Duration = Duration::from_secs(12);

pub fn serve(origin: &str) {
    let mut input = io::stdin().lock();
    let Some(first) = read_native(&mut input) else { return };
    let home = lt_proto::home();
    let Some(token) = lt_proto::key_token(&home) else { return };
    let Some(hello) = hello(&first, &token, origin, process::id()) else { return };
    let Some(link) = connect(&home) else { return };
    if link.send(&hello).is_err() {
        return;
    }
    let link = Arc::new(link);
    let down = Arc::clone(&link);
    thread::spawn(move || {
        let mut out = io::stdout().lock();
        while let Ok(msg) = down.recv() {
            if write_native(&mut out, &msg).is_err() {
                break;
            }
        }
        process::exit(0);
    });
    while let Some(msg) = read_native(&mut input) {
        if link.send(&msg).is_err() {
            break;
        }
    }
    process::exit(0);
}

fn hello(first: &[u8], token: &str, origin: &str, pid: u32) -> Option<Vec<u8>> {
    let Ok(Value::Object(mut msg)) = serde_json::from_slice::<Value>(first) else { return None };
    if msg.is_empty() {
        return None;
    }
    msg.insert("type".into(), json!("hello"));
    msg.insert("token".into(), json!(token));
    msg.insert("origin".into(), json!(origin));
    msg.insert("hostpid".into(), json!(pid));
    Some(Value::Object(msg).to_string().into_bytes())
}

fn connect(home: &Path) -> Option<Link> {
    let busy_until = Instant::now() + BUSY_WAIT;
    let mut started: Option<Instant> = None;
    loop {
        match Link::connect(home) {
            Ok(link) => return Some(link),
            Err(Conn::Busy) if Instant::now() < busy_until => thread::sleep(Duration::from_millis(15)),
            Err(Conn::Busy) => return None,
            Err(Conn::Missing) => {
                let since = *started.get_or_insert_with(|| {
                    start_daemon();
                    Instant::now()
                });
                if since.elapsed() > BOOT_WAIT {
                    return None;
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

fn start_daemon() {
    let Some(exe) = v2::client_exe() else { return };
    let mut cmd = Command::new(exe);
    cmd.arg("ping").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    v2::hide(&mut cmd);
    let _ = cmd.status();
}

fn read_native(input: &mut impl Read) -> Option<Vec<u8>> {
    let mut head = [0u8; 4];
    input.read_exact(&mut head).ok()?;
    let size = u32::from_ne_bytes(head) as usize;
    if size > MAX_IN {
        return None;
    }
    let mut buf = vec![0u8; size];
    input.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn write_native(out: &mut impl Write, msg: &[u8]) -> io::Result<()> {
    let size = u32::try_from(msg.len()).map_err(|_| io::Error::other("message too big"))?;
    out.write_all(&size.to_ne_bytes())?;
    out.write_all(msg)?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn native_frames_round_trip() {
        let mut buf = Vec::new();
        write_native(&mut buf, b"{\"a\":1}").unwrap();
        write_native(&mut buf, b"[]").unwrap();
        assert_eq!(&buf[..4], 7u32.to_ne_bytes());
        let mut input = Cursor::new(buf);
        assert_eq!(read_native(&mut input).as_deref(), Some(&b"{\"a\":1}"[..]));
        assert_eq!(read_native(&mut input).as_deref(), Some(&b"[]"[..]));
        assert_eq!(read_native(&mut input), None);
        let mut cut = Cursor::new([&9u32.to_ne_bytes()[..], b"abc"].concat());
        assert_eq!(read_native(&mut cut), None);
        let mut huge = Cursor::new(u32::MAX.to_ne_bytes().to_vec());
        assert_eq!(read_native(&mut huge), None);
    }

    #[test]
    fn hello_adds_what_the_daemon_checks() {
        let raw = hello(br#"{"brand":"Brave","epoch":7}"#, "ab12", "chrome-extension://x/", 42).unwrap();
        let msg: Value = serde_json::from_slice(&raw).unwrap();
        assert_eq!(msg["type"], "hello");
        assert_eq!(msg["token"], "ab12");
        assert_eq!(msg["origin"], "chrome-extension://x/");
        assert_eq!(msg["hostpid"], 42);
        assert_eq!(msg["brand"], "Brave");
        assert_eq!(msg["epoch"], 7);
        assert_eq!(hello(b"{}", "t", "o", 1), None);
        assert_eq!(hello(b"[1]", "t", "o", 1), None);
        assert_eq!(hello(b"not json", "t", "o", 1), None);
    }
}

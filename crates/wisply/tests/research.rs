use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

const EXE: &str = env!("CARGO_BIN_EXE_wisply");

fn run(home: &Path, base: &str, args: &[&str]) -> (i32, String) {
    let out = Command::new(EXE)
        .args(args)
        .env("WISPLY_HOME", home)
        .env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }, home)
        .env("WISPLY_RESEARCH_BASE", base)
        .env("CLAUDE_CODE_SESSION_ID", "it-research")
        .env_remove("MSYSTEM")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

struct Gone(PathBuf);

impl Drop for Gone {
    fn drop(&mut self) {
        let _ = run(&self.0, "", &["stop"]);
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn page(title: &str, body: &str) -> String {
    format!("<html><head><title>{title}</title></head><body><main><h1>{title}</h1>{body}</main></body></html>")
}

fn answer(path: &str, port: u16) -> (u16, &'static str, String) {
    let (route, query) = path.split_once('?').unwrap_or((path, ""));
    let a = format!("http://127.0.0.1:{port}");
    let b = format!("http://localhost:{port}");
    let filler =
        "Region threads tick their own chunks and entities, so plugins must schedule work on the owning region.";
    match route {
        "/duckduckgo" => {
            let mut html = format!(
                r#"<a class="result__a" href="{a}/guide">Folia scheduling guide for plugins</a>
                <a class="result__a" href="{b}/forum">Forum thread about Folia schedulers</a>"#
            );
            if query.contains("folialib") {
                html.push_str(&format!(
                    r#"<a class="result__a" href="{b}/folialib">FoliaLib project page and docs</a>"#
                ));
            }
            (200, "text/html", html)
        }
        "/bing" => {
            (200, "text/html", format!(r#"<h2><a href="{a}/guide">Folia scheduling guide for plugins</a></h2>"#))
        }
        "/brave" | "/mojeek" => (403, "text/html", "no".into()),
        "/guide" => (
            200,
            "text/html",
            page(
                "Folia scheduling guide",
                &format!(
                    "<h2>Region scheduler</h2><p>Folia replaces the Bukkit scheduler with a region scheduler. Use FoliaLib 0.4.3 \
                     to wrap the global, region and entity schedulers in one API. {}</p><h2>Other</h2><p>{}</p>",
                    filler,
                    "Cooking recipes and garden tips that have nothing to do with the question at all. ".repeat(4)
                ),
            ),
        ),
        "/forum" => (
            200,
            "text/html",
            page(
                "Forum: Folia schedulers",
                &format!(
                    "<p>I moved my plugin to FoliaLib and the region scheduler; FoliaLib keeps one code path for Paper \
                     and Folia. {}</p>",
                    filler
                ),
            ),
        ),
        "/folialib" => (
            200,
            "text/html",
            page(
                "FoliaLib",
                &format!(
                    "<p>FoliaLib is a wrapper for the Folia region scheduler. It also covers async tasks, teleports and delayed entity work on Paper and Folia servers alike. {}</p>",
                    filler
                ),
            ),
        ),
        "/wikipedia-en" | "/wikipedia-de" => (200, "application/json", r#"{"query":{"search":[]}}"#.into()),
        _ => (200, "application/json", "{}".into()),
    }
}

fn web(asked: Arc<Mutex<Vec<String>>>) -> u16 {
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = server.local_addr().unwrap().port();
    thread::spawn(move || {
        for link in server.incoming().flatten() {
            let asked = asked.clone();
            thread::spawn(move || {
                let mut reader = BufReader::new(link.try_clone().unwrap());
                let mut first = String::new();
                let _ = reader.read_line(&mut first);
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
                    line.clear();
                }
                let path = first.split_whitespace().nth(1).unwrap_or("/").to_string();
                asked.lock().unwrap().push(path.clone());
                let (status, ctype, body) = answer(&path, port);
                let head = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: {ctype}; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let mut link = link;
                let _ = link.write_all(head.as_bytes());
                let _ = link.write_all(body.as_bytes());
            });
        }
    });
    port
}

#[test]
fn research_searches_reads_ranks_and_saves() {
    let home = std::env::temp_dir().join(format!("wisply-it-research-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("v3"), "").unwrap();
    let guard = Gone(home.clone());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let base = format!("http://127.0.0.1:{}", web(asked.clone()));

    let (code, out) = run(&home, &base, &["research", "folia region scheduler", "folia scheduler plugin"]);
    assert_eq!(code, 0, "{out}");
    assert!(out.starts_with("research \"folia region scheduler\" + \"folia scheduler plugin\""), "{out}");
    assert!(out.contains("2 rounds"), "{out}");
    assert!(out.contains("[1] Folia scheduling guide"), "{out}");
    assert!(out.contains("> Folia replaces the Bukkit scheduler"), "{out}");
    assert!(!out.contains("Cooking recipes"), "{out}");
    assert!(out.contains("said by several sources:") && out.contains("FoliaLib ("), "{out}");
    assert!(out.contains("brave blocked"), "{out}");
    assert!(out.contains("saved: vault Research/"), "{out}");
    let paths = asked.lock().unwrap().clone();
    assert!(paths.iter().any(|p| p.starts_with("/duckduckgo") && p.contains("folialib")), "{paths:?}");
    assert!(paths.iter().any(|p| p == "/folialib"), "{paths:?}");
    let blocked = fs::read_to_string(home.join("research/engines.json")).unwrap();
    assert!(blocked.contains("brave") && blocked.contains("mojeek"), "{blocked}");
    let saved: Vec<_> = fs::read_dir(home.join("vault/Research")).unwrap().flatten().collect();
    assert_eq!(saved.len(), 1);
    let note = fs::read_to_string(saved[0].path()).unwrap();
    assert!(note.contains("type: research") && note.contains("[1] Folia scheduling guide"), "{note}");

    let before = asked.lock().unwrap().len();
    let (code, again) = run(&home, &base, &["research", "folia scheduler plugin", "folia region scheduler"]);
    assert_eq!(code, 0, "{again}");
    assert!(again.contains("saved earlier today"), "{again}");
    assert_eq!(asked.lock().unwrap().len(), before);

    let (code, quick) = run(&home, &base, &["research", "folia region scheduler", "--quick", "--fresh"]);
    assert_eq!(code, 0, "{quick}");
    assert!(quick.contains("(1 round,"), "{quick}");
    assert!(!quick.contains("brave"), "{quick}");
    drop(guard);
}

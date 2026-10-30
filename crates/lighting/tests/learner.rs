use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

const EXE: &str = env!("CARGO_BIN_EXE_lighting");

fn run(home: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new(EXE)
        .args(args)
        .env("LIGHTING_HOME", home)
        .env(if cfg!(windows) { "USERPROFILE" } else { "HOME" }, home)
        .env_remove("GEMINI_API_KEY")
        .env("CLAUDE_CODE_SESSION_ID", "it-learn")
        .env_remove("MSYSTEM")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

struct Gone(PathBuf);

impl Drop for Gone {
    fn drop(&mut self) {
        let _ = run(&self.0, &["stop"]);
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn model(asked: Arc<Mutex<Vec<Value>>>) -> u16 {
    let server = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = server.local_addr().unwrap().port();
    thread::spawn(move || {
        for link in server.incoming().flatten() {
            let mut reader = BufReader::new(link.try_clone().unwrap());
            let (mut line, mut size) = (String::new(), 0usize);
            while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
                if let Some(n) = line.to_lowercase().strip_prefix("content-length:") {
                    size = n.trim().parse().unwrap_or(0);
                }
                line.clear();
            }
            let mut body = vec![0; size];
            let _ = reader.read_exact(&mut body);
            asked.lock().unwrap().push(serde_json::from_slice(&body).unwrap_or(Value::Null));
            let answer = json!({"title": "Cart work", "summary": "- moved the cart to redis", "why": "faster",
                "rejected": "", "next": "write tests", "lessons": [],
                "notes": [{"kind": "decision", "title": "cart uses redis", "body": "redis keeps carts across restarts"}]});
            let reply = json!({"choices": [{"message": {"content": answer.to_string()}}]}).to_string();
            let mut link = link;
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                reply.len()
            );
            let _ = link.write_all(head.as_bytes()).and_then(|()| link.write_all(reply.as_bytes()));
        }
    });
    port
}

#[test]
fn sessions_are_learned_into_notes_and_vault_pages() {
    let base = if cfg!(windows) { std::env::temp_dir() } else { PathBuf::from("/tmp") };
    let home = base.join(format!("ltd-learn-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("v3"), "").unwrap();
    let _gone = Gone(home.clone());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let port = model(Arc::clone(&asked));
    let cfg =
        json!({"memory_api": format!("http://127.0.0.1:{port}/v1"), "memory_api_model": "brain", "gemini": false});
    fs::write(home.join("config.json"), cfg.to_string()).unwrap();
    let words = "please move the shopping cart storage over to redis so carts survive restarts ".repeat(12);
    let lines = [
        json!({"type": "ai-title", "aiTitle": "Shop cart"}),
        json!({"type": "user", "message": {"content": words}}),
        json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "Moving it now."}]}}),
    ];
    let transcript = home.join("t.jsonl");
    fs::write(&transcript, lines.iter().map(|l| l.to_string() + "\n").collect::<String>()).unwrap();
    let end = json!({"session_id": "abcdef123456", "transcript_path": transcript.display().to_string(), "cwd": home.display().to_string()});
    assert!(run(&home, &["memory"]).1.starts_with("memory on: "));
    assert_eq!(run(&home, &["hook", "end", &end.to_string()]).0, 0);
    let (code, said) = run(&home, &["memory", "learn"]);
    assert!(
        code == 0
            && said.starts_with("learning ")
            && said.ends_with("sessions in the background (lighting memory shows the state)"),
        "{said}"
    );
    let mut found = String::new();
    for _ in 0..100 {
        found = run(&home, &["recall", "redis", "cart"]).1;
        if found.contains("decision: cart uses redis") {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    assert!(found.contains("decision: cart uses redis"), "{found}\n{}", run(&home, &["memory"]).1);
    let sent = asked.lock().unwrap()[0].clone();
    assert_eq!(sent["model"], "brain");
    let user = sent["messages"][1]["content"].as_str().unwrap_or("");
    assert!(
        user.contains("Session title: Shop cart") && user.contains("USER: please move the shopping cart"),
        "{user}"
    );
    let sessions = home.join("vault").join("Sessions").join("general");
    let page = fs::read_dir(&sessions).unwrap().flatten().next().unwrap().path();
    let text = fs::read_to_string(&page).unwrap();
    assert!(
        text.starts_with("# Shop cart\nSession: abcdef12") && text.contains("## Done\n- moved the cart to redis"),
        "{text}"
    );
    let state = run(&home, &["memory"]).1;
    assert!(state.contains("learns with: brain at 127.0.0.1:") && state.contains("0 sessions to learn"), "{state}");
}

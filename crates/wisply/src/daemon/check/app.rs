use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use super::super::desk::probe;
use super::Run;
use super::web::grab;
use crate::testapp::{APP_TITLE, CHAT_TITLE};

const APPEAR: Duration = Duration::from_secs(8);
const STEP: Duration = Duration::from_millis(100);

fn close_all(title: &str) {
    for h in probe::titled(title) {
        probe::close(h);
    }
}

fn start(title: &str, chat: bool) -> bool {
    close_all(title);
    let Ok(exe) = std::env::current_exe() else { return false };
    let mut cmd = Command::new(exe);
    cmd.arg("testapp").args(chat.then_some("chat")).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    crate::v2::hide(&mut cmd);
    if cmd.spawn().is_err() {
        return false;
    }
    let end = Instant::now() + APPEAR;
    while Instant::now() < end {
        if !probe::titled(title).is_empty() {
            return true;
        }
        thread::sleep(STEP);
    }
    false
}

fn reset(r: &mut Run, title: &str) {
    close_all(title);
    r.ctx.desk.remove("hwnd");
    r.ctx.target = None;
}

pub fn desktop(r: &mut Run) {
    if !start(APP_TITLE, false) {
        r.skip("desktop tests (the test app did not open)");
        return;
    }
    let (before, t_before) = (probe::cursor(), Instant::now());
    let app = format!("app:{APP_TITLE}");
    let out = r.step("snap test app", &["snap", &app], |o, e| !e && o.contains("edit") && o.contains("checkbox"));
    let checkbox = grab(r"(?m)^(d\d+) checkbox", &out, "d0");
    r.step("type by field name", &["type", "Name:", "Claude"], |o, e| !e && o.contains("background"));
    r.step("toggle checkbox", &["click", &checkbox], |_, e| !e);
    r.step("click button by name", &["click", "Go"], |_, e| !e);
    r.step("read status via uia", &["snap", &app, "--text", "-f", "status"], |o, _| {
        o.contains("clicked 1 with Claude")
    });
    r.step("wait in an app", &["wait", "clicked 1", &app, "--timeout", "5s"], |o, e| !e && o.starts_with("ok ("));
    r.step("app wait gives up", &["wait", "never shown", &app, "--timeout", "1s"], |o, e| e && o.contains("not there"));
    r.step("tab key in background", &["press", &app, "ctrl+tab"], |o, e| {
        !e && o.contains("tab 2/3 \"Two\" selected via UI Automation")
    });
    r.step("tab number in background", &["press", &app, "ctrl+alt+3"], |o, e| !e && o.contains("tab 3/3 \"Three\""));
    let seen = r.step("ocr read", &["read", &app], |o, e| !e && o.contains("Remember"));
    let at = grab(r#"(?m)^o\d+ "Go" @(\d+,\d+)$"#, &seen, "");
    if at.is_empty() {
        r.skip("click a screen point (the screen text has no Go button)");
    } else {
        r.step("click a screen point", &["click", &format!("@{at}")], |o, e| !e && o.contains("clicked at"));
        r.step("screen point hit Go", &["snap", &app, "--text", "-f", "status"], |o, _| o.contains("clicked 2"));
        r.step("click a bare point", &["click", &at], |o, e| !e && o.contains("clicked at"));
        r.step("bare point hit Go", &["snap", &app, "--text", "-f", "status"], |o, _| o.contains("clicked 3"));
    }
    let front = probe::foreground();
    r.step("app that jumps to the front", &["click", "Front"], |o, e| !e && !o.contains("could not put"));
    r.check("previous window back in front", |_, _| {
        let now = probe::foreground();
        (now == front || !probe::titled(APP_TITLE).contains(&now), format!("in front {now:#x}, before {front:#x}"))
    });
    r.check("ocr via PrintWindow", |_, _| match probe::titled(APP_TITLE).first().map(|h| probe::ocr_text(*h)) {
        Some(Ok(text)) => (text.contains("Remember"), text),
        Some(Err(e)) => (false, e),
        None => (false, "test app window gone".into()),
    });
    let moved = probe::cursor() != before;
    if moved && u128::from(probe::idle_ms()) < t_before.elapsed().as_millis() {
        r.skip("mouse untouched (you used mouse or keyboard during the test)");
    } else {
        r.step("mouse untouched", &["ping"], |_, _| !moved);
    }
    r.step("record start (app)", &["record", "start", "selftest-app", "--desktop", "--injected"], |_, e| !e);
    r.step("recorded app click", &["click", "Go", "--mouse"], |_, e| !e);
    r.step("record stop (app)", &["record", "stop"], |o, e| !e && o.contains("click Go"));
    r.step("remove app routine", &["routine", "rm", "selftest-app"], |_, e| !e);
    r.step("window shot", &["shot", &app], |o, e| !e && o.contains(".jpg"));
    let wref = grab(r"^\[(w\d+)\]", &out, "w0");
    r.step("close app window", &["close", &wref], |o, e| !e && o.contains("closed"));
    reset(r, APP_TITLE);
}

pub fn chat(r: &mut Run) {
    if !start(CHAT_TITLE, true) {
        r.skip("chat tests (the test chat did not open)");
        return;
    }
    r.step("chat inbox quiet", &["inbox", CHAT_TITLE, "--timeout", "1s"], |o, e| {
        !e && o.starts_with("nothing new in 1 s")
    });
    r.step(
        "chat reply --wait (race)",
        &["reply", CHAT_TITLE, "hello from selftest", "--wait", "--timeout", "6s"],
        |o, e| !e && o.starts_with("sent to") && o.contains("peer: ok, 19 chars") && !o.contains("me: hello"),
    );
    r.step("chat link needs --yes", &["reply", CHAT_TITLE, "see https://example.com"], |o, e| e && o.contains("--yes"));
    r.step("chat inbox after answer", &["inbox", CHAT_TITLE, "--timeout", "1s"], |o, e| {
        !e && o.starts_with("nothing new")
    });
    r.step("chat unread", &["unread", CHAT_TITLE], |o, e| {
        !e && o.contains("DM Friends: 3 new") && o.contains("@ General: 2 mentions") && o.contains("unread: News")
    });
    reset(r, CHAT_TITLE);
}

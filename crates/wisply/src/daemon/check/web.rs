use std::fs;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use regex::Regex;
use serde_json::Value;

use super::Run;
use super::serve::Server;

const SEARCH_PROBE: &str = "[location.href, document.visibilityState, document.hasFocus(), document.activeElement ? document.activeElement.outerHTML.slice(0, 60) : '-', (document.querySelector('input[name=q]') || {}).value].join(' | ')";
const SETTLE: Duration = Duration::from_millis(500);

pub fn has(re: &str, text: &str) -> bool {
    Regex::new(re).is_ok_and(|r| r.is_match(text))
}

pub fn grab(re: &str, text: &str, default: &str) -> String {
    Regex::new(re)
        .ok()
        .and_then(|r| r.captures(text))
        .and_then(|c| c.get(1))
        .map_or(default.into(), |m| m.as_str().to_string())
}

pub fn tab_ref(out: &str) -> String {
    grab(r"^\[(t\d+)\]", out, "t0")
}

pub fn ok(_: &str, e: bool) -> bool {
    !e
}

pub fn forget_host(home: &Path, file: &str) {
    let path = home.join(file);
    let Some(mut data) = fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else {
        return;
    };
    if let Some(map) = data.as_object_mut() {
        map.remove("127.0.0.1");
    }
    if let Ok(text) = serde_json::to_string_pretty(&data) {
        let _ = fs::write(path, text);
    }
}

pub fn browser(r: &mut Run, srv: &Server) -> String {
    let base = srv.base.as_str();
    let at = |p: &str| format!("{base}{p}");
    let decoy = r.step("decoy tab", &["open", &at("second"), "--new"], |o, e| !e && o.contains("Second"));
    r.step("open fixture", &["open", base, "--new"], |o, e| !e && o.contains("Wisply Fixture"));
    r.step("open with a filter", &["open", base, "--new", "-f", "Sign in"], |o, e| {
        !e && o.contains("Sign in") && !o.contains("Dark mode") && !o.contains("nav ")
    });
    r.step("close filtered tab", &["close"], ok);
    r.step("open --text", &["open", &at("second"), "--new", "--text"], |o, e| {
        !e && o.starts_with("[t") && o.contains("Opened in a new tab") && !o.contains("\ne1 ")
    });
    r.step("close text tab", &["close"], ok);
    r.step("empty page shows text", &["open", &at("quiet"), "--new"], |o, e| {
        !e && o.contains("(no controls in view) text: Only text here")
    });
    r.step("close quiet tab", &["close"], ok);
    srv.visits.store(0, Ordering::SeqCst);
    r.step("count page", &["open", &at("count"), "--new"], |o, e| !e && o.contains("Count"));
    r.step("wait --reload", &["wait", "Visit 2", "--reload", "3s", "--timeout", "15s"], |o, e| {
        !e && o.starts_with("ok after 1 reload")
    });
    r.step("close count tab", &["close"], ok);
    r.step("finder page", &["open", &at("finder"), "--new"], |o, e| !e && o.contains("Finder"));
    let got = r.step("search learns the URL", &["search", "wisply"], |o, e| !e && o.contains("learned the search URL"));
    if !got.contains("learned the search URL") {
        r.diag(&["tabs"]);
        r.diag(&["js", SEARCH_PROBE]);
    } else if got.contains("did not reach the page") {
        r.rows.push(format!(
            "DIAG {:<26} | {}",
            "search Enter fallback",
            got.replace('\n', " / ").chars().take(300).collect::<String>()
        ));
    }
    r.step("search jumps directly", &["search", "second try"], |o, e| {
        !e && o.contains("find?q=second+try") && !o.contains("(typed")
    });
    r.step("jumped to results", &["expect", "Found: second try"], ok);
    r.step("close finder tab", &["close"], ok);
    forget_host(&r.d.home, "sites.json");
    r.step("page loads JSON", &["open", &at("data"), "--new"], |o, e| {
        !e && (o.contains("data: 1 JSON call (wisply net)") || o.contains("loading"))
    });
    r.step("JSON arrives", &["wait", "Items 40"], ok);
    r.step("net lists it", &["net"], |o, e| !e && has(r"n1 GET 127\.0\.0\.1:\d+/api/items \(json \d+ KB\)", o));
    r.step("net shows its shape", &["net", "1"], |o, e| {
        !e && o.contains("items[] (40) {id: 0") && o.contains("total: 40")
    });
    r.step("webmcp tools", &["tools"], |o, e| !e && (o.contains("no WebMCP tools") || o.contains('(')));
    r.step("close data tab", &["close"], ok);
    let fixture = base.to_string();
    r.check("sessions stay apart", |d, _| super::web2::sessions_check(d, &fixture));
    r.check("closed page reopens for reading", |d, _| super::web2::reopen_check(d, &fixture));
    r.check("a loop is braked", |d, _| super::web2::brake_check(d));
    r.step("hidden text not shown", &["snap", "--all"], |o, e| {
        !e && !o.contains("IGNORE ALL") && !o.contains("HIDDEN INJECTION")
    });
    r.step("nav collapsed", &["snap", "--force"], |o, e| {
        !e && o.contains("nav ") && o.contains("items (wisply snap -s")
    });
    r.step("header collapsed, names kept", &["snap", "--force"], |o, e| {
        !e && has(r"header: 13 items \(wisply snap -s e\d+\): Top 1 \| Top 2", o) && !o.contains("link \"Top 5\"")
    });
    r.step("footnotes, key hints, hashes", &["snap", "--all"], |o, e| {
        !e && !o.contains("\"[1]\"") && o.contains("link \"Keyboard help\" ->") && o.contains("commit/0123456…")
    });
    r.step("obvious link target hidden", &["snap", "--all", "--force"], |o, e| {
        !e && has(r#"(?m)link "Pricing"$"#, o) && o.contains("link \"Open in new tab\" ->")
    });
    r.step("snap --links shows it", &["snap", "--all", "--links"], |o, e| {
        !e && o.contains("link \"Pricing\" ->/pricing")
    });
    r.step("long list collapsed", &["snap", "--force"], |o, _| o.contains("similar items") || o.contains("more:"));
    r.step("cookie banner noticed", &["snap", "--force"], |o, _| o.contains("cookie banner"));
    r.step("dismiss cookie banner", &["dismiss"], |o, e| !e && o.contains("rejected"));
    r.step("toggle shows [ ]", &["snap", "-f", "Dark mode"], |o, e| !e && o.contains("\"Dark mode\" [ ]"));
    r.step("toggle click", &["click", "Dark mode"], ok);
    r.step("toggle shows [x]", &["snap", "-f", "Dark mode"], |o, e| !e && o.contains("\"Dark mode\" [x]"));
    r.step("click -f shows the result", &["click", "Dark mode", "-f", "Dark mode"], |o, e| {
        !e && o.starts_with("ok") && o.contains("\n[t") && o.contains("\"Dark mode\" [ ]")
    });
    r.step("click back", &["click", "Dark mode"], ok);
    r.step("keys from the page", &["keys"], |o, e| {
        !e && o.contains("ctrl+k") && o.contains("Command menu") && o.contains("g k")
    });
    forget_host(&r.d.home, "keys.json");
    r.step("search box", &["search", "wisply docs"], ok);
    r.step("search submitted", &["expect", "Searched: wisply docs"], ok);
    r.step("filter a|b", &["snap", "-f", "Sign in|Open modal"], |o, e| {
        !e && o.contains("Sign in") && o.contains("Open modal")
    });
    r.step("filter looks past the view", &["snap", "-f", "Sign in|Item 40"], |o, e| {
        !e && o.contains("Sign in")
            && o.contains("link \"Item 40\"")
            && (o.contains("outside the view") || o.contains("no match in view"))
    });
    r.step("filter names text and misses", &["snap", "-f", "Sign in|Fact|zqxj"], |o, e| {
        !e && o.contains("Sign in") && o.contains("\"Fact") && o.contains("no match for \"zqxj\" on the whole page")
    });
    r.step("nameless button hint", &["snap", "--all"], |o, e| !e && o.contains("button #nameless-save"));
    r.step("click by #hint", &["click", "#nameless-save"], ok);
    r.step("hint click worked", &["expect", "Saved via hint"], ok);
    let vout = r.step("video line", &["snap", "-f", "video"], |o, e| {
        !e && o.contains("video \"demo clip\" 0:00") && o.contains("paused")
    });
    let vref = grab(r"(?m)^(e\d+) video", &vout, "e0");
    r.step("images with --media", &["snap", "--media", "-f", "orange test square"], |o, e| {
        !e && o.contains("img \"orange test square\" 64x64")
    });
    r.step("frames of a paused video", &["frames", &vref, "--count", "2", "--every", "400"], |o, e| {
        !e && o.contains(".jpg") && o.contains("did not move") && o.contains("not seekable")
    });
    r.step("frames --small", &["frames", &vref, "--count", "2", "--every", "400", "--small"], |o, e| {
        !e && o.contains("(768x")
    });
    r.step("frames read captions", &["frames", &vref, "--count", "2", "--every", "300"], |o, e| {
        !e && o.contains("captions:") && o.contains("Hello from the caption track")
    });
    let out = r.d.home.join("out");
    let _ = fs::create_dir_all(&out);
    let pic = out.join("selftest.png");
    let _ = image::RgbImage::from_pixel(40, 20, image::Rgb([255, 165, 0])).save(&pic);
    let pic = pic.display().to_string();
    r.step("read an image file", &["read", &pic], |o, e| !e && o.contains("(image)") && o.contains("40x20"));
    r.step("page changes by itself", &["js", "setTimeout(() => history.pushState({}, '', '?moved=1'), 50); 1"], ok);
    thread::sleep(SETTLE);
    r.step("stale text click refused", &["click", "#nameless-save"], |o, e| e && o.contains("page changed"));
    r.step("next click works", &["click", "#nameless-save"], ok);
    tab_ref(&decoy)
}

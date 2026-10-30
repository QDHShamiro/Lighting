use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

const FIXTURE: &[u8] = include_bytes!("../../../assets/fixture.html");
const DEMO: &[u8] = include_bytes!("../../../assets/demo.html");
const SLOW_SCRIPT: Duration = Duration::from_secs(14);
const ITEMS: usize = 40;
const EXTRA: &[(&str, &str)] = &[
    ("/frame", "<title>Frame</title><button onclick=\"this.textContent='Frame clicked'\">Frame button</button>"),
    ("/second", "<title>Second</title><h1>Second page</h1><p>Opened in a new tab.</p>"),
    ("/quiet", "<title>Quiet</title><p>Only text here, nothing to click.</p>"),
    (
        "/finder",
        "<title>Finder</title><form action=/find method=get role=search><input name=q aria-label=Search></form>",
    ),
    (
        "/data",
        "<title>Data</title><p id=n>loading</p><script>fetch('/api/items').then((r) => r.json()).then((d) => { document.getElementById('n').textContent = 'Items ' + d.items.length; })</script>",
    ),
    (
        "/slow",
        "<title>Slow</title><script defer src=/slow.js></script><h1>Slow page</h1><p>Its script is still coming.</p>",
    ),
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Site {
    Fixture,
    Demo,
}

pub struct Server {
    pub base: String,
    pub visits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    port: u16,
}

impl Server {
    pub fn start(site: Site) -> std::io::Result<Server> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let visits = Arc::new(AtomicUsize::new(0));
        let (flag, count) = (Arc::clone(&stop), Arc::clone(&visits));
        thread::spawn(move || {
            for conn in listener.incoming() {
                if flag.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(conn) = conn else { continue };
                let count = Arc::clone(&count);
                thread::spawn(move || answer(conn, site, &count));
            }
        });
        Ok(Server { base: format!("http://127.0.0.1:{port}/"), visits, stop, port })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#x27;")
}

fn unquote(s: &str) -> String {
    let bytes = s.replace('+', " ").into_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let Some(b) =
                bytes.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok())
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn items() -> String {
    let rows: Vec<String> = (0..ITEMS)
        .map(|i| format!("{{\"id\": {i}, \"name\": \"Item {i}\", \"price\": {:?}}}", i as f64 * 1.5))
        .collect();
    format!("{{\"items\": [{}], \"total\": {ITEMS}}}", rows.join(", "))
}

pub fn pdf(text: &str) -> Vec<u8> {
    let stream = format!("BT /F1 18 Tf 20 40 Td ({text}) Tj ET");
    let objs = [
        "<</Type/Catalog/Pages 2 0 R>>".to_string(),
        "<</Type/Pages/Kids[3 0 R]/Count 1>>".into(),
        "<</Type/Page/Parent 2 0 R/MediaBox[0 0 300 100]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>".into(),
        format!("<</Length {}>>stream\n{stream}\nendstream", stream.len()),
        "<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>".into(),
    ];
    let mut out = String::from("%PDF-1.4\n");
    let mut offsets = Vec::new();
    for (i, obj) in objs.iter().enumerate() {
        offsets.push(out.len());
        out += &format!("{} 0 obj{obj}endobj\n", i + 1);
    }
    let xref = out.len();
    out += &format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1);
    for o in offsets {
        out += &format!("{o:010} 00000 n \n");
    }
    out += &format!("trailer<</Size {}/Root 1 0 R>>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1);
    out.into_bytes()
}

fn route(target: &str, site: Site, visits: &AtomicUsize) -> Option<(Vec<u8>, &'static str)> {
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let html = "text/html; charset=utf-8";
    if site == Site::Demo {
        return matches!(path, "/" | "/index.html").then(|| (DEMO.to_vec(), html));
    }
    Some(match path {
        "/count" => {
            let n = visits.fetch_add(1, Ordering::SeqCst) + 1;
            (format!("<!doctype html><title>Count</title><p>Visit {n}</p>").into_bytes(), html)
        }
        "/api/items" => (items().into_bytes(), "application/json"),
        "/find" => {
            let q = query.split('&').find_map(|kv| kv.strip_prefix("q=")).map(unquote).unwrap_or_default();
            (format!("<!doctype html><title>Found</title><p>Found: {}</p>", escape(&q)).into_bytes(), html)
        }
        "/" | "/index.html" => (FIXTURE.to_vec(), html),
        "/doc.pdf" => (pdf("Hello Lighting PDF"), "application/pdf"),
        "/slow.js" => {
            thread::sleep(SLOW_SCRIPT);
            (b"document.title = 'Slow done';".to_vec(), "text/javascript")
        }
        _ => {
            let (_, body) = EXTRA.iter().find(|(p, _)| *p == path)?;
            (format!("<!doctype html><meta charset=utf-8>{body}").into_bytes(), html)
        }
    })
}

fn answer(conn: TcpStream, site: Site, visits: &AtomicUsize) {
    let mut reader = BufReader::new(&conn);
    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return;
    }
    let mut line = String::new();
    while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
        line.clear();
    }
    let target = first.split_whitespace().nth(1).unwrap_or("/");
    let mut conn = &conn;
    let _ = match route(target, site, visits) {
        Some((body, kind)) => conn
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: {kind}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                )
                .as_bytes(),
            )
            .and_then(|()| conn.write_all(&body)),
        None => conn.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"),
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn get(base: &str, path: &str) -> String {
        let addr = base.trim_start_matches("http://").trim_end_matches('/');
        let mut s = TcpStream::connect(addr).unwrap();
        s.write_all(format!("GET {path} HTTP/1.1\r\nhost: x\r\n\r\n").as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }

    #[test]
    fn the_fixture_server_answers_like_the_2x_one() {
        let srv = Server::start(Site::Fixture).unwrap();
        assert!(get(&srv.base, "/").contains("Lighting Fixture"));
        assert!(get(&srv.base, "/count").contains("Visit 1") && get(&srv.base, "/count?x").contains("Visit 2"));
        assert!(get(&srv.base, "/find?q=second+try%21").contains("Found: second try!"));
        assert!(get(&srv.base, "/find?q=%3Cb%3E").contains("Found: &lt;b&gt;"));
        assert!(
            get(&srv.base, "/api/items").contains("\"total\": 40")
                && items().contains("\"name\": \"Item 39\", \"price\": 58.5")
        );
        assert!(get(&srv.base, "/second").contains("Opened in a new tab."));
        assert!(get(&srv.base, "/nope").starts_with("HTTP/1.1 404"));
        let doc = pdf("Hi");
        assert!(doc.starts_with(b"%PDF-1.4") && doc.ends_with(b"%%EOF\n"));
        let demo = Server::start(Site::Demo).unwrap();
        assert!(get(&demo.base, "/").contains("Lighting demo") && get(&demo.base, "/x").starts_with("HTTP/1.1 404"));
    }
}

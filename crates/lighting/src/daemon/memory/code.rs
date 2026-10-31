use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use lt_code::Index;
use lt_core::{Call, Fail};
use serde_json::{Map, Value, json};

use super::super::ctx::Ctx;
use super::super::{Daemon, game, web};
use super::embed;

const REFRESH: Duration = Duration::from_secs(2);
const BG_REFRESH: Duration = Duration::from_secs(20);
const QUERY_BUDGET: Duration = Duration::from_secs(4);
const EMBED_PASS: Duration = Duration::from_secs(50);
const EMBED_WAIT: Duration = Duration::from_secs(30);
const EMBED_BATCH: usize = 32;
const ACTIVE_DAYS: f64 = 7.0;
const IMPACT_LINES: usize = 60;
const CODE_CHARS: usize = 6000;
const ROOTS: &str = "roots.json";
const BG_EVERY: Duration = Duration::from_secs(600);
const BG_RECENT_S: f64 = 3600.0;

static LAST_BG: Mutex<Option<Instant>> = Mutex::new(None);

thread_local! {
    static OPEN: RefCell<HashMap<PathBuf, Index>> = RefCell::new(HashMap::new());
}

fn roots_file(home: &Path) -> PathBuf {
    home.join("code").join(ROOTS)
}

fn active(home: &Path) -> Map<String, Value> {
    let now = lt_memory::now();
    let all: Map<String, Value> =
        fs::read_to_string(roots_file(home)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    all.into_iter().filter(|(_, at)| at.as_f64().is_some_and(|t| now - t < ACTIVE_DAYS * 86400.0)).collect()
}

fn used(home: &Path, root: &Path) {
    let mut all = active(home);
    all.insert(root.to_string_lossy().into_owned(), json!(lt_memory::now()));
    let _ = fs::create_dir_all(home.join("code"));
    let _ = fs::write(roots_file(home), Value::Object(all).to_string());
}

fn with_index<T>(home: &Path, root: &Path, f: impl FnOnce(&mut Index) -> Result<T, String>) -> Result<T, Fail> {
    OPEN.with(|cell| {
        let mut open = cell.borrow_mut();
        if !open.contains_key(root) {
            let index = Index::open(home, root).map_err(|e| Fail::new(format!("cannot open the code map ({e})")))?;
            open.insert(root.to_path_buf(), index);
        }
        let Some(index) = open.get_mut(root) else { return Err(Fail::new("the code map is not open")) };
        f(index).map_err(|e| Fail::new(format!("code map: {e}")))
    })
}

fn meaning(home: &Path, index: &Index, text: &str) -> Result<String, String> {
    let total = index.meant_total().map_err(|e| e.to_string())?;
    let none = format!(
        "the meaning index of {} is not built yet ({total} symbols wait; it builds in the background)",
        index.root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    );
    match embed::embed(home, &[text.to_string()], true, QUERY_BUDGET) {
        Ok(mut vecs) if !vecs.is_empty() => {
            let q = vecs.remove(0);
            let found = index.meaning(&q, "").map_err(|e| e.to_string())?;
            if found.is_empty() { index.by_name(text, &none).map_err(|e| e.to_string()) } else { Ok(found) }
        }
        Ok(_) => index.by_name(text, &none).map_err(|e| e.to_string()),
        Err(why) => index.by_name(text, &format!("no meaning search right now ({why})")).map_err(|e| e.to_string()),
    }
}

fn answer(home: &Path, index: &Index, c: &Call) -> Result<String, String> {
    let db = |e: lt_code::DbError| e.to_string();
    let pos = &c.pos;
    let sub = pos.first().map(|p| p.to_lowercase()).unwrap_or_default();
    match (sub.as_str(), pos.len()) {
        (_, 0) => index.overview().map_err(db),
        ("callers", 2) => index.callers(&pos[1]).map_err(db),
        ("outline", 2) => index.outline(&pos[1]).map_err(db),
        ("body", 2) => index.body(&pos[1]),
        ("impact", 1) => index.impact(c.val("base"), c.on("tests")),
        _ => {
            let text = pos.join(" ");
            if text.trim().contains(' ') {
                meaning(home, index, text.trim())
            } else {
                index.lookup(text.trim()).map_err(db)
            }
        }
    }
}

pub fn code(d: &Daemon, ctx: &mut Ctx, c: &Call) -> Result<String, Fail> {
    let place = c.val("project").map(String::from).unwrap_or_else(|| ctx.cwd.clone());
    let Some(root) = lt_memory::root_of(Path::new(&place)) else {
        return Err(Fail::hint(
            format!("no project here ({place})"),
            "run it inside a project folder, or wisply code ... --project <path>",
        ));
    };
    used(&d.home, &root);
    let (text, left) = with_index(&d.home, &root, |index| {
        index.refresh(Some(REFRESH)).map_err(|e| e.to_string())?;
        Ok((answer(&d.home, index, c)?, index.left))
    })?;
    let note = if left > 0 {
        format!("\n({left} changed files are not read yet, the next wisply code reads on)")
    } else {
        String::new()
    };
    let lines = (c.pos.first().map(String::as_str) == Some("impact")).then_some(IMPACT_LINES);
    Ok(web::capped(d, ctx, &format!("{text}{note}"), "code", lines, CODE_CHARS))
}

pub fn background(home: &Path) {
    {
        let Ok(mut last) = LAST_BG.lock() else { return };
        if last.is_some_and(|t| t.elapsed() < BG_EVERY) {
            return;
        }
        *last = Some(Instant::now());
    }
    let end = Instant::now() + EMBED_PASS;
    let now = lt_memory::now();
    let recent = active(home).into_iter().filter(|(_, at)| at.as_f64().is_some_and(|t| now - t < BG_RECENT_S));
    for root in recent.map(|(p, _)| PathBuf::from(p)).filter(|p| p.is_dir()) {
        let Ok(mut index) = Index::open(home, &root) else { continue };
        if index.refresh(Some(BG_REFRESH)).is_err() {
            continue;
        }
        while Instant::now() < end && game::running().is_empty() {
            let Ok(todo) = index.unembedded(EMBED_BATCH) else { break };
            if todo.is_empty() {
                break;
            }
            let texts: Vec<String> = todo.iter().map(|(_, t)| t.clone()).collect();
            let Ok(vecs) = embed::embed(home, &texts, false, EMBED_WAIT) else { return };
            let pairs: Vec<(i64, Vec<f32>)> = todo.iter().map(|(id, _)| *id).zip(vecs).collect();
            if index.save_vecs(&pairs).is_err() {
                break;
            }
        }
    }
}

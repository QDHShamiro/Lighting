use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::files;
use crate::run::{Run, int, text};

pub const BACKUP_MAX_MB: u64 = 200;

fn read_json(f: &Path) -> Value {
    fs::read_to_string(f).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
}

fn write_json(f: &Path, v: &Value) -> Result<(), String> {
    if let Some(d) = f.parent() {
        fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    fs::write(f, serde_json::to_string_pretty(v).unwrap_or_default()).map_err(|e| e.to_string())
}

fn secs() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

pub fn digest(p: &Path) -> String {
    let Ok(raw) = fs::read(p) else { return String::new() };
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in &raw {
        h = (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3);
    }
    format!("{h:016x}-{}", raw.len())
}

pub fn backup(run: &Run) -> Value {
    let cap = BACKUP_MAX_MB * 1024 * 1024;
    let list = files::walk(&run.paths());
    let mut total = 0u64;
    for f in &list {
        total += fs::metadata(f).map_or(0, |m| m.len());
        if total > cap {
            return json!({"ok": false, "mb": total / 1024 / 1024, "files": list.len()});
        }
    }
    let dest = run.dir.join("backup");
    let mut manifest = Vec::new();
    for (i, f) in list.iter().enumerate() {
        let bak = format!("{i}.bak");
        if fs::create_dir_all(&dest).and_then(|()| fs::copy(f, dest.join(&bak))).is_ok() {
            manifest.push(json!({"abs": f.to_string_lossy(), "bak": bak}));
        }
    }
    let n = manifest.len();
    let _ = write_json(&dest.join("manifest.json"), &Value::Array(manifest));
    json!({"ok": true, "mb": total / 1024 / 1024, "files": n})
}

fn cp_dir(run: &Run, id: i64) -> PathBuf {
    run.dir.join("cp").join(id.to_string())
}

fn load_cp(dir: &Path) -> Value {
    match read_json(&dir.join("manifest.json")) {
        Value::Object(m) => Value::Object(m),
        _ => json!({"files": [], "reverted": false}),
    }
}

fn entries(cp: &Value) -> Vec<Value> {
    cp.get("files").and_then(Value::as_array).cloned().unwrap_or_default()
}

pub fn checkpoint(run: &Run, id: i64, list: &[PathBuf]) -> Result<(Vec<String>, Vec<String>), String> {
    let dir = cp_dir(run, id);
    let mut cp = load_cp(&dir);
    if cp.get("reverted").and_then(Value::as_bool) == Some(true) {
        cp = json!({"files": []});
    }
    cp["reverted"] = json!(false);
    let known: BTreeSet<String> = entries(&cp).iter().map(|e| text(e, "abs").to_string()).collect();
    let (mut added, mut kept) = (Vec::new(), Vec::new());
    let mut all = entries(&cp);
    for f in list {
        let abs = f.to_string_lossy().to_string();
        let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if known.contains(&abs) {
            kept.push(name);
            continue;
        }
        if f.is_dir() {
            return Err(format!("{abs} is a folder; checkpoint single files"));
        }
        let mut entry = json!({"abs": abs, "existed": f.exists(), "t": secs()});
        if f.exists() {
            let bak = format!("{}.bak", all.len());
            fs::create_dir_all(&dir)
                .and_then(|()| fs::copy(f, dir.join(&bak)))
                .map_err(|e| format!("cannot back up {name}: {e}"))?;
            entry["bak"] = json!(bak);
            added.push(name);
        } else {
            added.push(format!("{name} (new)"));
        }
        all.push(entry);
    }
    cp["files"] = Value::Array(all);
    write_json(&dir.join("manifest.json"), &cp)?;
    Ok((added, kept))
}

pub fn mark_after(run: &Run, id: i64) {
    let dir = cp_dir(run, id);
    if !dir.join("manifest.json").exists() {
        return;
    }
    let mut cp = load_cp(&dir);
    let marked: Vec<Value> = entries(&cp)
        .into_iter()
        .map(|mut e| {
            e["after"] = json!(digest(Path::new(text(&e, "abs"))));
            e
        })
        .collect();
    cp["files"] = Value::Array(marked);
    let _ = write_json(&dir.join("manifest.json"), &cp);
}

fn checkpoints(run: &Run) -> Vec<(i64, PathBuf, Value)> {
    let Ok(list) = fs::read_dir(run.dir.join("cp")) else { return Vec::new() };
    let mut out: Vec<(i64, PathBuf, Value)> = list
        .flatten()
        .filter_map(|e| {
            let id: i64 = e.file_name().to_string_lossy().parse().ok()?;
            let cp = load_cp(&e.path());
            Some((id, e.path(), cp))
        })
        .collect();
    let first =
        |cp: &Value| entries(cp).iter().filter_map(|e| e.get("t").and_then(Value::as_f64)).fold(f64::MAX, f64::min);
    out.sort_by(|a, b| first(&a.2).total_cmp(&first(&b.2)));
    out
}

fn changed_since(dir: &Path, e: &Value) -> bool {
    let f = Path::new(text(e, "abs"));
    if e.get("existed").and_then(Value::as_bool) != Some(true) {
        return f.exists();
    }
    !f.exists() || fs::read(f).ok() != fs::read(dir.join(text(e, "bak"))).ok()
}

fn conflicts(run: &Run, id: i64, cp: &Value) -> Vec<String> {
    let name =
        |e: &Value| Path::new(text(e, "abs")).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let mine: BTreeMap<String, f64> = entries(cp)
        .iter()
        .map(|e| (text(e, "abs").to_string(), e.get("t").and_then(Value::as_f64).unwrap_or(0.0)))
        .collect();
    let mut out: Vec<String> = entries(cp)
        .iter()
        .filter(|e| e.get("after").is_some() && digest(Path::new(text(e, "abs"))) != text(e, "after"))
        .map(|e| format!("{} (changed afterwards)", name(e)))
        .collect();
    for (other, dir, ocp) in checkpoints(run) {
        if other == id || ocp.get("reverted").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        for e in entries(&ocp) {
            let t = e.get("t").and_then(Value::as_f64).unwrap_or(0.0);
            if mine.get(text(&e, "abs")).is_some_and(|m| t > *m) && changed_since(&dir, &e) {
                out.push(format!("{} (#{other})", name(&e)));
            }
        }
    }
    out
}

pub fn revert(run: &Run, id: i64, force: bool) -> Result<(Vec<String>, Vec<String>), String> {
    let dir = cp_dir(run, id);
    if !dir.join("manifest.json").exists() {
        return Err(format!("no checkpoint for #{id}"));
    }
    let mut cp = load_cp(&dir);
    let later = conflicts(run, id, &cp);
    if !later.is_empty() && !force {
        return Err(format!(
            "#{id} not reverted, changed later: {}. Revert the later ideas first or solve it by hand (--force overwrites)",
            later.join(", ")
        ));
    }
    let mut done = Vec::new();
    for e in entries(&cp) {
        let f = PathBuf::from(text(&e, "abs"));
        let name = f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if e.get("existed").and_then(Value::as_bool) == Some(true) {
            if let Some(d) = f.parent() {
                let _ = fs::create_dir_all(d);
            }
            fs::copy(dir.join(text(&e, "bak")), &f).map_err(|err| format!("cannot restore {name}: {err}"))?;
            done.push(format!("{name} back"));
        } else if f.exists() {
            let _ = fs::remove_file(&f);
            done.push(format!("{name} deleted (was new)"));
        }
    }
    cp["reverted"] = json!(true);
    write_json(&dir.join("manifest.json"), &cp)?;
    Ok((done, later))
}

fn originals(run: &Run) -> (BTreeMap<String, PathBuf>, Vec<String>) {
    let bdir = run.dir.join("backup");
    let mut orig: BTreeMap<String, PathBuf> = BTreeMap::new();
    if let Value::Array(list) = read_json(&bdir.join("manifest.json")) {
        for e in list {
            orig.insert(text(&e, "abs").to_string(), bdir.join(text(&e, "bak")));
        }
    }
    let mut created = BTreeSet::new();
    for (_, dir, cp) in checkpoints(run) {
        for e in entries(&cp) {
            let abs = text(&e, "abs").to_string();
            if orig.contains_key(&abs) || created.contains(&abs) {
                continue;
            }
            if e.get("existed").and_then(Value::as_bool) == Some(true) {
                orig.insert(abs, dir.join(text(&e, "bak")));
            } else {
                created.insert(abs);
            }
        }
    }
    (orig, created.into_iter().collect())
}

pub fn restore_all(run: &Run) -> Result<(usize, usize), String> {
    let (orig, created) = originals(run);
    if orig.is_empty() && created.is_empty() {
        return Err("no start backup in this run".into());
    }
    for (abs, bak) in &orig {
        if let Some(d) = Path::new(abs).parent() {
            let _ = fs::create_dir_all(d);
        }
        fs::copy(bak, abs).map_err(|e| format!("cannot restore {abs}: {e}"))?;
    }
    let removed = created.iter().filter(|a| fs::remove_file(a).is_ok()).count();
    Ok((orig.len(), removed))
}

fn git_diff(old: Option<&Path>, new: &Path, empty: &Path) -> String {
    let old = old.unwrap_or(empty);
    let new_or_empty = if new.exists() { new } else { empty };
    crate::measure::quiet(&mut Command::new("git"))
        .args(["diff", "--no-index", "--no-color", "-U3", "--"])
        .arg(old)
        .arg(new_or_empty)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

pub fn diff(run: &Run, label: &str) -> Result<(PathBuf, usize, usize, usize), String> {
    let (orig, created) = originals(run);
    let mut chunks = Vec::new();
    if !orig.is_empty() || !created.is_empty() {
        let empty = run.dir.join("empty.txt");
        let _ = fs::write(&empty, "");
        for (abs, bak) in &orig {
            if fs::read(abs).ok() == fs::read(bak).ok() {
                continue;
            }
            let d = git_diff(Some(bak), Path::new(abs), &empty);
            chunks.push(d.replace(
                &files::slashed(bak),
                &format!("before/{}", Path::new(abs).file_name().map(|n| n.to_string_lossy()).unwrap_or_default()),
            ));
        }
        for abs in &created {
            chunks.push(git_diff(None, Path::new(abs), &empty));
        }
    } else if run.s.pointer("/git/repo").and_then(Value::as_bool) == Some(true) {
        let out = crate::measure::quiet(&mut Command::new("git"))
            .arg("-C")
            .arg(run.root())
            .arg("diff")
            .output()
            .map_err(|e| e.to_string())?;
        chunks.push(String::from_utf8_lossy(&out.stdout).to_string());
    } else {
        return Err("no backup and no git, so no diff".into());
    }
    let body = chunks.join("\n");
    let lines: Vec<&str> = body.lines().collect();
    let plus = lines.iter().filter(|l| l.starts_with('+') && !l.starts_with("+++")).count();
    let minus = lines.iter().filter(|l| l.starts_with('-') && !l.starts_with("---")).count();
    let n = lines.iter().filter(|l| l.starts_with("diff --git")).count();
    let out = run.dir.join(format!("diff-{label}.txt"));
    fs::write(&out, body).map_err(|e| e.to_string())?;
    Ok((out, n, plus, minus))
}

pub fn blind(run: &mut Run, before: &Path, after: &Path) -> Result<Value, String> {
    for f in [before, after] {
        if !f.is_file() {
            return Err(format!("{} does not exist", f.display()));
        }
    }
    let n = run.list("blind").len() as i64 + 1;
    let dir = run.dir.join("blind").join(n.to_string());
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let flip = (secs() * 1000.0) as u64 % 2 == 1;
    let order = if flip { [("after", after), ("before", before)] } else { [("before", before), ("after", after)] };
    let ext = after.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_else(|| "txt".into());
    let mut paths = serde_json::Map::new();
    for (k, (_, src)) in order.iter().enumerate() {
        let tgt = dir.join(format!("version{}.{ext}", k + 1));
        fs::copy(src, &tgt).map_err(|e| e.to_string())?;
        paths.insert((k + 1).to_string(), json!(files::slashed(&tgt)));
    }
    let entry = json!({
        "n": n, "map": {"1": order[0].0, "2": order[1].0}, "before": files::slashed(before),
        "after": files::slashed(after), "paths": paths, "at": crate::run::now()
    });
    run.list("blind").push(entry.clone());
    Ok(entry)
}

pub fn blind_result(run: &mut Run, n: i64, verdict: &str) -> Result<String, String> {
    let list = run.list("blind");
    let e = list.iter_mut().find(|e| int(e, "n") == n).ok_or(format!("blind comparison {n} does not exist"))?;
    let winner = if verdict == "same" { "same".to_string() } else { text(&e["map"], verdict).to_string() };
    if winner.is_empty() {
        return Err("verdict is 1, 2 or same".into());
    }
    e["winner"] = json!(winner);
    Ok(winner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::Home;

    #[test]
    fn checkpoints_revert_and_guard_later_changes() {
        let base = std::env::temp_dir().join(format!("wisply-think-safety-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let work = base.join("work");
        fs::create_dir_all(&work).unwrap();
        let a = work.join("a.txt");
        fs::write(&a, "one").unwrap();
        let home = Home::new(&base, "s");
        let mut run = Run::create(&home, "safety");
        run.map().insert("paths".into(), json!([work.to_string_lossy()]));
        assert_eq!(backup(&run)["files"], 1);
        let fresh = work.join("new.txt");
        checkpoint(&run, 1, &[a.clone(), fresh.clone()]).unwrap();
        fs::write(&a, "two").unwrap();
        fs::write(&fresh, "x").unwrap();
        mark_after(&run, 1);
        checkpoint(&run, 2, std::slice::from_ref(&a)).unwrap();
        fs::write(&a, "three").unwrap();
        assert!(revert(&run, 1, false).unwrap_err().contains("a.txt"));
        revert(&run, 2, false).unwrap();
        assert_eq!(fs::read_to_string(&a).unwrap(), "two");
        let (done, _) = revert(&run, 1, false).unwrap();
        assert_eq!(done, ["a.txt back", "new.txt deleted (was new)"]);
        assert_eq!(fs::read_to_string(&a).unwrap(), "one");
        fs::write(&a, "four").unwrap();
        assert_eq!(restore_all(&run).unwrap(), (1, 0));
        assert_eq!(fs::read_to_string(&a).unwrap(), "one");
        let b = work.join("b.txt");
        fs::write(&b, "two").unwrap();
        let e = blind(&mut run, &a, &b).unwrap();
        let after_slot = if e["map"]["1"] == "after" { "1" } else { "2" };
        assert_eq!(blind_result(&mut run, 1, after_slot).unwrap(), "after");
        let _ = fs::remove_dir_all(&base);
    }
}

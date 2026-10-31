use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

use serde_json::{Value, json};
use wisply_core::Call;

use crate::files::{self, Pack};
use crate::fix::{self, Plan};
use crate::rules::{self, Finding, Scope};
use crate::run::{self, Home, Run, int, text};
use crate::{Env, measure, personal, safety, skill};

const SCAN_SHOWN: usize = 25;

fn findings(env: &Env, r: &Run) -> (Vec<Finding>, BTreeSet<Pack>) {
    let root = r.root();
    let list = files::walk(&r.paths());
    let packs = env.packs(&root, &list);
    let mine = personal::load(&env.user, &root, &env.profile_packs());
    let scope = Scope { root: &root, packs: &packs, rules: &mine };
    (rules::scan(&scope, &list), packs)
}

fn as_idea(f: &Finding) -> Value {
    json!({"title": format!("{}: {}", f.rule, f.why), "lens": f.pack.name(), "tool": f.rule, "impact": f.impact,
           "sure": f.sure, "effort": f.effort, "kind": "safe", "by": "wisply",
           "place": f.place(), "fix": f.fix, "fixer": f.fixer})
}

pub fn scan(env: &Env, home: &Home, c: &Call) -> Result<String, String> {
    let mut r = home.load(c.val("run"))?;
    if r.paths().is_empty() {
        return Err("this run has no path; start it with one: wisply think start \"<name>\" <path>".into());
    }
    let (found, packs) = findings(env, &r);
    let room = if c.on("all") { usize::MAX } else { (int(&r.s, "idea_goal").max(10) / 2) as usize };
    let (mut added, mut known) = (0, 0);
    for f in &found {
        if added >= room {
            break;
        }
        let place = f.place();
        if r.ideas().iter().any(|i| text(i, "place") == place && text(i, "tool") == f.rule) {
            known += 1;
            continue;
        }
        if r.add_idea(as_idea(f), true).is_ok() {
            added += 1;
        }
    }
    r.map().insert("packs".into(), json!(packs.iter().map(|p| p.name()).collect::<Vec<_>>()));
    r.save()?;
    let mut by: BTreeMap<&str, usize> = BTreeMap::new();
    for f in &found {
        *by.entry(f.pack.name()).or_default() += 1;
    }
    let counts: Vec<String> = by.iter().map(|(k, v)| format!("{k} {v}")).collect();
    let fixable = found.iter().filter(|f| f.fixer.is_some()).count();
    let mut out = vec![format!(
        "[scan] {} findings ({}) | checks: {} | {fixable} Wisply can fix itself",
        found.len(),
        if counts.is_empty() { "none".to_string() } else { counts.join(", ") },
        crate::flow::packs_line(&packs)
    )];
    for f in found.iter().take(SCAN_SHOWN) {
        out.push(format!(
            "  {} {}: {} -> {}{}",
            f.place(),
            f.rule,
            f.why,
            f.fix,
            if f.fixer.is_some() { " [fix]" } else { "" }
        ));
    }
    if found.len() > SCAN_SHOWN {
        out.push(format!("  ... +{} more (wisply think idea list --all)", found.len() - SCAN_SHOWN));
    }
    out.push(format!(
        "+{added} ideas from wisply{}{}",
        if known > 0 { format!(", {known} already listed") } else { String::new() },
        if found.len() > added + known {
            format!(", {} left out (--all adds them)", found.len() - added - known)
        } else {
            String::new()
        }
    ));
    if fixable > 0 {
        out.push("next: wisply think fix (backs up, fixes, proves with the build, reverts when worse)".into());
    }
    Ok(out.join("\n"))
}

fn build_proof(env: &Env, r: &Run) -> Option<Value> {
    let root = r.root();
    let (build, _) = measure::commands(text(&r.s, "kind"), &root);
    let cmd = build?;
    Some(measure_once(env, r, &cmd, 1))
}

fn measure_once(env: &Env, r: &Run, cmd: &str, runs: usize) -> Value {
    let root = r.root();
    match cmd {
        "lint-skill" => {
            let (errs, warns) = skill::lint(&root);
            let mut v = json!({"cmd": cmd, "exit": i32::from(!errs.is_empty()), "errors": errs.len(), "warnings": warns.len(), "at": run::now()});
            if !errs.is_empty() || !warns.is_empty() {
                v["tail"] = json!([errs, warns].concat().join("\n"));
            }
            v
        }
        "selftest" => {
            let script = root.join("scripts").join("selftest.py");
            if !script.exists() {
                return json!({"skipped": "no scripts/selftest.py", "cmd": cmd});
            }
            let ran = measure::run(&format!("python -B \"{}\"", script.display()), &root, measure::TIMEOUT);
            measure::result(cmd, &[ran])
        }
        _ => {
            let _ = env;
            let mut all = Vec::new();
            for _ in 0..runs.max(1) {
                let ran = measure::run(cmd, &root, measure::TIMEOUT);
                let failed = ran.code != 0;
                all.push(ran);
                if failed {
                    break;
                }
            }
            measure::result(cmd, &all)
        }
    }
}

fn ok(m: &Value) -> bool {
    m.get("exit").and_then(Value::as_i64) == Some(0)
}

pub fn fix(env: &Env, home: &Home, c: &Call) -> Result<String, String> {
    let mut r = home.load(c.val("run"))?;
    if r.paths().is_empty() {
        return Err("this run has no path to fix".into());
    }
    let (found, packs) = findings(env, &r);
    let root = r.root();
    let plans: Vec<Plan> = fix::plans(&root, &found, &packs, c.on("deep"));
    if plans.is_empty() {
        return Ok("nothing Wisply can fix itself; the findings are ideas for Claude (wisply think idea list)".into());
    }
    let before_ok = r.s.pointer("/measures/before").is_none_or(ok);
    let mut out = Vec::new();
    for plan in plans {
        let id = r.add_idea(
            json!({"title": plan.title, "lens": "fix", "tool": plan.fixer, "impact": 1, "sure": 3, "effort": 1,
                   "kind": "safe", "by": "wisply", "fixer": plan.fixer}),
            true,
        )?;
        r.save()?;
        if let Err(e) = safety::checkpoint(&r, id, &plan.files) {
            set(&mut r, id, "dropped", "why", &e);
            out.push(format!("#{id} {}: not started ({e})", plan.title));
            continue;
        }
        let did = match fix::apply(&plan, &root) {
            Ok(d) => d,
            Err(e) => {
                let _ = safety::revert(&r, id, true);
                set(&mut r, id, "dropped", "why", &e);
                out.push(format!("#{id} {}: failed, reverted ({e})", plan.title));
                continue;
            }
        };
        let proof = if plan.fixer == "png" { None } else { build_proof(env, &r) };
        match proof {
            Some(m) if m.get("skipped").is_some() => {
                set(&mut r, id, "unchecked", "proof", &did);
                safety::mark_after(&r, id);
                out.push(format!("#{id} {}: {did}; build not run ({}) -> unchecked", plan.title, text(&m, "skipped")));
            }
            Some(m) if !ok(&m) && before_ok => {
                let _ = safety::revert(&r, id, true);
                let why = format!("build broke after the fix: {}", measure::line(&m));
                set(&mut r, id, "dropped", "why", &why);
                out.push(format!("#{id} {}: {why} -> reverted", plan.title));
            }
            Some(m) => {
                let p = format!("{did}; {}: {}", text(&m, "cmd"), measure::line(&m));
                set(&mut r, id, "done", "proof", &p);
                safety::mark_after(&r, id);
                covered(&mut r, id, plan.fixer);
                out.push(format!("#{id} {}: {p}", plan.title));
            }
            None => {
                set(&mut r, id, "done", "proof", &did);
                safety::mark_after(&r, id);
                covered(&mut r, id, plan.fixer);
                out.push(format!("#{id} {}: {did}", plan.title));
            }
        }
    }
    r.save()?;
    out.push("next: wisply think diff, then review the change".into());
    Ok(out.join("\n"))
}

fn covered(r: &mut Run, by: i64, fixer: &str) {
    let ids: Vec<i64> = r
        .ideas()
        .iter()
        .filter(|i| int(i, "id") != by && text(i, "fixer") == fixer && text(i, "status") == "new")
        .map(|i| int(i, "id"))
        .collect();
    for id in ids {
        set(r, id, "done", "proof", &format!("fixed by #{by}"));
    }
}

fn set(r: &mut Run, id: i64, status: &str, key: &str, val: &str) {
    let round = r.round();
    if let Some(i) = r.idea_mut(id) {
        i["status"] = json!(status);
        i[key] = json!(val);
        i["round_status"] = json!(round);
    }
    let _ = r.save();
}

pub fn measure_cmd(env: &Env, home: &Home, c: &Call) -> Result<String, String> {
    let mut r = home.load(c.val("run"))?;
    let label =
        c.pos.get(1).cloned().ok_or("wisply think measure <label> [--cmd \"...\"] [--runs 5] [--test] [--metrics]")?;
    let root = r.root();
    let (build, test) = measure::commands(text(&r.s, "kind"), &root);
    let cmd = c.val("cmd").map(str::to_string).or(if c.on("test") { test } else { build });
    let res = match cmd {
        None => json!({"skipped": format!("no check command for project kind '{}', use --cmd", text(&r.s, "kind"))}),
        Some(cmd) => {
            if root.as_os_str().is_empty() {
                return Err("this run has no project folder; use --cmd with a run that has a path".into());
            }
            let runs = c.val("runs").and_then(|n| n.parse().ok()).unwrap_or(1);
            measure_once(env, &r, &cmd, runs)
        }
    };
    let mut out = vec![format!("{label}: {}", measure::line(&res))];
    if let Some(t) = res.get("tail").and_then(Value::as_str) {
        out.push(t.to_string());
    }
    r.s["measures"][&label] = res;
    if c.on("metrics") && !r.paths().is_empty() {
        let m = measure::metrics(&r.paths());
        out.push(format!("metrics: {}", measure::metrics_line(&m)));
        r.s["metrics"][&label] = m;
    }
    r.save()?;
    Ok(out.join("\n"))
}

pub fn compare(home: &Home, c: &Call) -> Result<String, String> {
    let r = home.load(c.val("run"))?;
    let a = c.pos.get(1).map_or("before", String::as_str);
    let b = c.pos.get(2).map_or("after", String::as_str);
    let m = |l: &str| r.s.pointer(&format!("/measures/{l}")).cloned().unwrap_or(Value::Null);
    let mut out = measure::compare(&m(a), &m(b));
    let mt = |l: &str| r.s.pointer(&format!("/metrics/{l}")).cloned();
    if let (Some(x), Some(y)) = (mt(a), mt(b)) {
        out.push_str(&format!(
            "\nlines: {} -> {} | files: {} -> {} | complexity max: {} -> {} | over 10: {} -> {}",
            x["lines"], y["lines"], x["files"], y["files"], x["ccn_max"], y["ccn_max"], x["ccn_over"], y["ccn_over"]
        ));
    }
    Ok(out)
}

pub fn checkpoint(env: &Env, home: &Home, c: &Call) -> Result<String, String> {
    let r = home.load(c.val("run"))?;
    let id: i64 = c
        .pos
        .get(1)
        .and_then(|s| s.trim_start_matches('#').parse().ok())
        .ok_or("wisply think checkpoint <idea id> <files...>")?;
    let list: Vec<PathBuf> = c.pos.iter().skip(2).map(|p| env.path(p)).collect();
    if list.is_empty() {
        return Err("which files? wisply think checkpoint <id> <files...>".into());
    }
    let (added, kept) = safety::checkpoint(&r, id, &list)?;
    Ok(format!(
        "checkpoint #{id}: {}{}",
        if added.is_empty() { "nothing new".to_string() } else { added.join(", ") },
        if kept.is_empty() { String::new() } else { format!(" | already saved: {}", kept.join(", ")) }
    ))
}

pub fn revert(home: &Home, c: &Call) -> Result<String, String> {
    let mut r = home.load(c.val("run"))?;
    let id: i64 = c
        .pos
        .get(1)
        .and_then(|s| s.trim_start_matches('#').parse().ok())
        .ok_or("wisply think revert <idea id> --why \"...\"")?;
    let (done, later) = safety::revert(&r, id, c.on("force"))?;
    let why = c.val("why").unwrap_or("not better");
    set(&mut r, id, "dropped", "why", why);
    let mut out = format!(
        "#{id} reverted: {} -> dropped",
        if done.is_empty() { "nothing to do".to_string() } else { done.join(", ") }
    );
    if !later.is_empty() {
        out.push_str(&format!("\nWARNING overwritten (--force): {} - check those ideas again", later.join(", ")));
    }
    Ok(out)
}

pub fn restore(home: &Home, c: &Call) -> Result<String, String> {
    let r = home.load(c.val("run"))?;
    if !c.on("yes") {
        return Err("puts EVERYTHING back to the start. Only when the user says so: wisply think restore --yes".into());
    }
    let (n, removed) = safety::restore_all(&r)?;
    Ok(format!("start state restored: {n} files, {removed} new files removed"))
}

pub fn diff(home: &Home, c: &Call) -> Result<String, String> {
    let r = home.load(c.val("run"))?;
    let label = c.pos.get(1).cloned().unwrap_or_else(|| format!("r{}", r.round()));
    let (file, n, plus, minus) = safety::diff(&r, &label)?;
    Ok(format!(
        "diff: {} ({n} files, +{plus} -{minus}) - read it line by line against the checklist",
        files::slashed(&file)
    ))
}

pub fn save(env: &Env, home: &Home, c: &Call) -> Result<String, String> {
    let r = home.load(c.val("run"))?;
    let label = c.pos.get(1).ok_or("wisply think save <label> [--file f] (or the text on stdin)")?;
    let dir = r.dir.join("text");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let ext = c.val("ext").unwrap_or("md").trim_start_matches('.').to_string();
    let target = dir.join(format!("{}.{ext}", run::slug(label, 40)));
    if let Some(f) = c.val("file") {
        fs::copy(env.path(f), &target).map_err(|e| format!("cannot copy {f}: {e}"))?;
    } else {
        let body = c.val("input").unwrap_or("");
        if body.trim().is_empty() {
            return Err("text missing: pipe it in (<<'EOF') or use --file <path>".into());
        }
        fs::write(&target, body).map_err(|e| e.to_string())?;
    }
    Ok(format!("version saved: {}", files::slashed(&target)))
}

pub fn blind(env: &Env, home: &Home, c: &Call) -> Result<String, String> {
    let mut r = home.load(c.val("run"))?;
    let (Some(a), Some(b)) = (c.pos.get(1), c.pos.get(2)) else {
        return Err("wisply think blind <before file> <after file>".into());
    };
    let e = safety::blind(&mut r, &env.path(a), &env.path(b))?;
    r.save()?;
    Ok(format!(
        "blind comparison {} ready: the user picks in the Wisply window (wisply ui -> Blind), or in the chat:\n  version 1 = {}\n  version 2 = {}\nnever say which one is new; then: wisply think blind-result {} 1|2|same",
        e["n"],
        text(&e["paths"], "1"),
        text(&e["paths"], "2"),
        e["n"]
    ))
}

pub fn blind_result(home: &Home, c: &Call) -> Result<String, String> {
    let mut r = home.load(c.val("run"))?;
    let n: i64 = c.pos.get(1).and_then(|s| s.parse().ok()).ok_or("wisply think blind-result <n> 1|2|same")?;
    let verdict = c.pos.get(2).map(String::as_str).unwrap_or("");
    if !["1", "2", "same"].contains(&verdict) {
        return Err("verdict is 1, 2 or same".into());
    }
    let winner = safety::blind_result(&mut r, n, verdict)?;
    r.save()?;
    Ok(match winner.as_str() {
        "after" => "AFTER picked -> better".into(),
        "before" => "BEFORE picked -> revert the change".into(),
        _ => "TIE -> the original stays".into(),
    })
}

pub fn blind_open(home: &Home) -> Value {
    let mut out = Vec::new();
    for r in home.all() {
        for e in r.s.get("blind").and_then(Value::as_array).cloned().unwrap_or_default() {
            if e.get("winner").is_none() {
                out.push(json!({"run": r.id(), "name": text(&r.s, "name"), "n": e["n"], "paths": e["paths"]}));
            }
        }
    }
    Value::Array(out)
}

pub fn profile(env: &Env, c: &Call) -> Result<String, String> {
    let file = env.home.join("think").join("profile.json");
    let mut p: Value = fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(json!({}));
    match c.pos.get(1).map(String::as_str) {
        Some("set") => {
            let list: Vec<String> =
                c.pos[2..].join(",").split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect();
            let bad: Vec<&String> = list.iter().filter(|s| Pack::from_name(s).is_none()).collect();
            if !bad.is_empty() || list.is_empty() {
                let names: Vec<&str> = Pack::ALL.iter().map(|p| p.name()).collect();
                return Err(format!("packs are {}", names.join(", ")));
            }
            p["set"] = json!(list);
        }
        Some("auto") => {
            p.as_object_mut().map(|m| m.remove("set"));
        }
        _ => {}
    }
    let counts = env.person();
    p["counts"] =
        json!(counts.iter().map(|(k, v)| (k.name().to_string(), json!(v))).collect::<serde_json::Map<_, _>>());
    let _ = fs::create_dir_all(file.parent().unwrap_or(&env.home));
    fs::write(&file, serde_json::to_string_pretty(&p).unwrap_or_default()).map_err(|e| e.to_string())?;
    let total: usize = counts.values().sum::<usize>().max(1);
    let mut rows: Vec<(&Pack, &usize)> = counts.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1));
    let seen: Vec<String> = rows.iter().map(|(k, v)| format!("{} {} %", k.name(), **v * 100 / total)).collect();
    let set: Vec<&str> = p
        .get("set")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    Ok(format!(
        "you build: {}\nchecks used: {}\n(wisply think profile set web,rust limits the checks; profile auto follows your projects again)",
        if seen.is_empty() { "nothing seen yet".to_string() } else { seen.join(", ") },
        if set.is_empty() {
            "whatever each project contains (auto)".to_string()
        } else {
            format!("only {} + general code, docs, images", set.join(", "))
        }
    ))
}

pub fn rules_cmd(env: &Env) -> String {
    let root = env.cwd.clone();
    let list = personal::load(&env.user, &files::root_of(&root), &env.profile_packs());
    if list.is_empty() {
        return "no personal rules found (CLAUDE.md files and feedback memories are read)".into();
    }
    let mut out = vec![format!("{} personal rules Wisply checks:", list.len())];
    out.extend(list.iter().map(|r| format!("  {}", personal::describe(r))));
    out.join("\n")
}

pub fn lint_skill(env: &Env, c: &Call) -> Result<String, String> {
    let dir = env.path(c.pos.get(1).ok_or("wisply think lint-skill <skill folder>")?);
    let (errs, warns) = skill::lint(&dir);
    let mut out: Vec<String> = errs.iter().map(|e| format!("ERROR: {e}")).collect();
    out.extend(warns.iter().map(|w| format!("WARNING: {w}")));
    out.push(if errs.is_empty() && warns.is_empty() {
        "OK".into()
    } else {
        format!("{} errors, {} warnings", errs.len(), warns.len())
    });
    Ok(out.join("\n"))
}

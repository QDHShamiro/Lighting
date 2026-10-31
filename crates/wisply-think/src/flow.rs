use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use wisply_core::Call;

use crate::files::{self, Pack};
use crate::run::{self, Home, Run, STATUSES, cut, int, points, table, text};
use crate::{Env, measure, safety};

pub const MAX_ROUNDS: i64 = 5;
const IMPORTANT: i64 = 2;
const REPORT_ROWS: usize = 10;
const TOP_DEFAULT: usize = 15;
const RUNS_KEEP: usize = 10;
const BACKUPS_KEEP: usize = 3;
const BRIEF_LESSONS: usize = 10;
const START_LESSONS: usize = 5;
const EXHAUSTED_RATIO: f64 = 0.5;
const EXHAUSTED_BATCH: usize = 4;

fn sizes(size: &str) -> (i64, i64) {
    match size {
        "file" => (20, 5),
        "feature" => (50, 12),
        _ => (100, 25),
    }
}

fn tags_for(kind: &str) -> &'static [&'static str] {
    match kind {
        "skill" => &["skill", "prompt", "protokoll", "protocol"],
        "maven" | "gradle" => &["java", "plugin", "code", "folia"],
        "npm" => &["web", "js", "ts", "code"],
        "python" => &["python", "tool", "code"],
        "cargo" => &["rust", "code", "tool"],
        _ => &["text", "plan", "design", "logik", "logic"],
    }
}

pub fn git_info(root: &Path) -> Value {
    let git = |args: &[&str]| -> Option<String> {
        let o = measure::quiet(&mut std::process::Command::new("git")).arg("-C").arg(root).args(args).output().ok()?;
        o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let Some(top) = git(&["rev-parse", "--show-toplevel"]) else { return json!({"repo": false}) };
    let dirty = git(&["status", "--porcelain"]).unwrap_or_default().lines().filter(|l| !l.is_empty()).count();
    json!({"repo": true, "top": top, "branch": git(&["branch", "--show-current"]).unwrap_or_default(),
           "head": git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "no commits".into()), "dirty": dirty})
}

fn git_hint(g: &Value, name: &str) -> String {
    let branch = text(g, "branch");
    let dirty = int(g, "dirty");
    if (branch == "main" || branch == "master") && dirty == 0 {
        format!("on {branch} and clean -> git checkout -b refactor/{} (fix/ for bug fixes)", run::slug(name, 24))
    } else if dirty > 0 {
        format!("{dirty} changes by someone else -> do not commit or stash them, checkpoints only")
    } else {
        "feature branch -> stay there".into()
    }
}

pub fn matching_lessons(home: &Home, kind: &str) -> Vec<String> {
    let tags = tags_for(kind);
    home.lessons().into_iter().filter(|l| tags.contains(&run::lesson_tag(l).as_str())).take(START_LESSONS).collect()
}

pub fn start(env: &Env, home: &Home, c: &Call) -> Result<String, String> {
    let name = c.pos.get(1).cloned().ok_or("wisply think start \"<name>\" [paths] [--goal \"...\"] [--quick]")?;
    let paths: Vec<PathBuf> = c.pos.iter().skip(2).map(|p| env.path(p)).collect();
    for p in &paths {
        if !p.exists() {
            return Err(format!("{} does not exist", p.display()));
        }
    }
    let quick = c.on("quick");
    let count = paths.first().map_or(0, |_| files::walk(&paths).len());
    let size = c.val("size").map(str::to_string).unwrap_or_else(|| {
        match count {
            0 | 1 => "file",
            2..=25 => "feature",
            _ => "project",
        }
        .to_string()
    });
    if let Ok(old) = home.load(None)
        && text(&old.s, "status") == "running"
    {
        env.note(format!("open run {} ({}) is paused, back with --run {}", old.id(), text(&old.s, "name"), old.id()));
    }
    let root = paths.first().map(|p| files::root_of(p));
    let kind = root.as_deref().map_or("", measure::kind_of);
    let (goal_first, _) = sizes(&size);
    let mut r = Run::create(home, &name);
    let packs: Vec<&str> = root
        .as_deref()
        .map(|rt| env.packs(rt, &files::walk(&paths)))
        .unwrap_or_default()
        .iter()
        .map(|p| p.name())
        .collect();
    let git = root.as_deref().map_or(json!({"repo": false}), git_info);
    for (k, v) in [
        ("goal", json!(c.val("goal").unwrap_or(&name))),
        ("paths", json!(paths.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>())),
        ("size", json!(size)),
        ("mode", json!(if quick { "quick" } else { "full" })),
        ("idea_goal", json!(if quick { 10 } else { goal_first })),
        ("max_rounds", json!(if quick { 1 } else { MAX_ROUNDS })),
        ("round", json!(1)),
        ("kind", json!(kind)),
        ("root", json!(root.as_deref().map(|r| r.to_string_lossy().to_string()).unwrap_or_default())),
        ("packs", json!(packs)),
        ("git", git.clone()),
    ] {
        r.map().insert(k.into(), v);
    }
    r.save()?;
    home.set_current(r.id());
    let bk = if paths.is_empty() { json!({"ok": false, "mb": 0, "files": 0}) } else { safety::backup(&r) };
    r.map().insert("backup".into(), bk.clone());
    r.save()?;
    let mut out = vec![
        format!("run {} ({})", r.id(), if quick { "quick" } else { "full" }),
        format!("size: {size} -> at least {} ideas, max {} rounds", int(&r.s, "idea_goal"), int(&r.s, "max_rounds")),
    ];
    if let Some(rt) = &root {
        let (build, test) = measure::commands(kind, rt);
        out.push(format!(
            "project: {} in {} | build: {} | test: {} | checks: {}",
            if kind.is_empty() { "unknown" } else { kind },
            files::slashed(rt),
            build.as_deref().unwrap_or("- (use --cmd)"),
            test.as_deref().unwrap_or("-"),
            if packs.is_empty() { "-".to_string() } else { packs.join(", ") }
        ));
    } else {
        out.push("no path: text or plan in the chat -> keep versions with wisply think save <label>".into());
    }
    out.push(if git["repo"] == true {
        format!(
            "git: {} @ {}, {} changed -> {}",
            text(&git, "branch"),
            text(&git, "head"),
            int(&git, "dirty"),
            git_hint(&git, &name)
        )
    } else {
        "git: none".into()
    });
    if !paths.is_empty() {
        out.push(if bk["ok"] == true {
            format!("backup: {} files, {} MB", bk["files"], bk["mb"])
        } else {
            format!("backup: TOO BIG (over {} MB) -> checkpoints per idea only", safety::BACKUP_MAX_MB)
        });
    }
    out.push(if env.playing.is_empty() {
        "game: none".into()
    } else {
        format!("game: {} runs (checks still run)", env.playing)
    });
    let lessons = matching_lessons(home, if kind.is_empty() { "text" } else { kind });
    if !lessons.is_empty() {
        out.push("matching lessons:".into());
        out.extend(lessons.iter().map(|l| format!("  {l}")));
    }
    out.push(
        "next: 2 understand -> wisply think crit \"<criterion>\" ...; wisply think scan finds problems itself".into(),
    );
    Ok(out.join("\n"))
}

pub fn crit(home: &Home, c: &Call) -> Result<String, String> {
    let mut r = home.load(c.val("run"))?;
    let new: Vec<String> = c.pos.iter().skip(1).map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).collect();
    if new.is_empty() {
        return Err("wisply think crit \"<criterion>\" ...".into());
    }
    let started = r.ideas().iter().any(|i| ["done", "unchecked"].contains(&text(i, "status")));
    let why = c.val("why");
    if started && why.is_none() {
        return Err(
            "criteria are fixed since step 2. Add only with a reason: wisply think crit \"<c>\" --why \"<reason>\""
                .into(),
        );
    }
    if let (true, Some(w)) = (started, why) {
        r.list("criteria_later").push(json!({"criteria": new, "why": w}));
    }
    let list = r.list("criteria");
    for n in new {
        if !list.iter().any(|x| x.as_str() == Some(&n)) {
            list.push(json!(n));
        }
    }
    let all: Vec<String> = list.iter().filter_map(Value::as_str).map(str::to_string).collect();
    r.save()?;
    Ok(format!("criteria: {}", all.join(" | ")))
}

pub fn hint(home: &Home, c: &Call) -> Result<String, String> {
    let mut r = home.load(c.val("run"))?;
    let t = c.pos[1..].join(" ");
    if t.trim().is_empty() {
        return Err("wisply think hint \"<doubt about the goal>\"".into());
    }
    r.list("hints").push(json!(t));
    let n = r.list("hints").len();
    r.save()?;
    Ok(format!("hint about the goal saved ({n})"))
}

pub fn source(home: &Home, c: &Call) -> Result<String, String> {
    let mut r = home.load(c.val("run"))?;
    if c.on("list") || c.pos.len() < 2 {
        let rows: Vec<String> =
            r.list("sources").iter().map(|s| format!("- {} - {}", text(s, "url"), text(s, "proof"))).collect();
        return Ok(if rows.is_empty() { "no sources".into() } else { rows.join("\n") });
    }
    let entry = json!({"url": c.pos[1], "proof": c.pos[2..].join(" ")});
    r.list("sources").push(entry);
    let n = r.list("sources").len();
    r.save()?;
    Ok(format!("source {n} saved"))
}

fn parse_idea(line: &str) -> Result<Value, String> {
    let mut parts: Vec<String> = line.split('|').map(|p| p.trim().to_string()).collect();
    if parts[0].is_empty() {
        return Err("title missing".into());
    }
    let defaults = ["", "-", "-", "2", "2", "2", "safe"];
    while parts.len() < 7 {
        parts.push(defaults[parts.len()].into());
    }
    let mut nums = Vec::new();
    for (name, v) in ["impact", "sure", "effort"].iter().zip(&parts[3..6]) {
        match v.as_str() {
            "1" | "2" | "3" => nums.push(v.parse::<i64>().unwrap_or(2)),
            _ => return Err(format!("{name} must be 1, 2 or 3, not '{v}'")),
        }
    }
    let kind = match parts[6].to_lowercase().as_str() {
        "safe" | "sicher" => "safe",
        "big" | "gross" | "groß" => "big",
        other => return Err(format!("kind must be safe or big, not '{other}'")),
    };
    Ok(json!({"title": parts[0], "lens": parts[1], "tool": parts[2], "impact": nums[0], "sure": nums[1],
              "effort": nums[2], "kind": kind, "by": "claude"}))
}

pub fn idea(home: &Home, c: &Call) -> Result<String, String> {
    let sub = c.pos.get(1).map(String::as_str).unwrap_or("list");
    let mut r = home.load(c.val("run"))?;
    match sub {
        "add" => {
            let mut lines: Vec<String> = c.pos.iter().skip(2).cloned().collect();
            if lines.is_empty() {
                lines = c.val("input").unwrap_or("").lines().map(str::to_string).collect();
            }
            lines.retain(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
            if lines.is_empty() {
                return Err(
                    "no ideas. Format: \"title | lens | tool | impact 1-3 | sure 1-3 | effort 1-3 | safe/big\"".into(),
                );
            }
            let (mut added, mut dupes, mut bad) = (0, Vec::new(), Vec::new());
            for l in &lines {
                match parse_idea(l) {
                    Ok(v) => match r.add_idea(v, c.on("force")) {
                        Ok(_) => added += 1,
                        Err(twin) => dupes.push(format!("{} ~ {twin}", cut(l.split('|').next().unwrap_or(""), 40))),
                    },
                    Err(e) => bad.push(format!("{}: {e}", cut(l, 40))),
                }
            }
            let exhausted =
                lines.len() >= EXHAUSTED_BATCH && dupes.len() as f64 / lines.len() as f64 >= EXHAUSTED_RATIO;
            if exhausted {
                let round = r.round();
                r.map().insert("ideas_out".into(), json!(round));
            }
            r.save()?;
            let mut out = vec![format!(
                "+{added} ideas (total {}/{}){}",
                r.ideas().len(),
                int(&r.s, "idea_goal"),
                if dupes.is_empty() { String::new() } else { format!(", {} duplicates", dupes.len()) }
            )];
            out.extend(dupes.iter().map(|d| format!("  duplicate: {d}")));
            out.extend(bad.iter().map(|b| format!("  ERROR: {b}")));
            if exhausted {
                out.push("ideas are running out (many duplicates): switch tool or role, or honestly fewer".into());
            }
            Ok(out.join("\n"))
        }
        "out" => {
            let why = c.pos[2..].join(" ");
            if why.trim().is_empty() {
                return Err("wisply think idea out \"<why no more real ideas come>\"".into());
            }
            let round = r.round();
            r.map().insert("ideas_out".into(), json!(round));
            r.s["ideas_out_why"][round.to_string()] = json!(why);
            r.save()?;
            Ok(format!(
                "round {round}: ideas honestly ran out ({} total), the reason goes into the report",
                r.ideas().len()
            ))
        }
        "set" => idea_set(&mut r, c),
        _ => Ok(idea_list(&r, c)),
    }
}

fn idea_list(r: &Run, c: &Call) -> String {
    let mut ideas: Vec<Value> = r.ideas();
    if let Some(st) = c.val("status") {
        ideas.retain(|i| text(i, "status") == st);
    }
    if let Some(n) = c.val("round").and_then(|n| n.parse::<i64>().ok()) {
        ideas.retain(|i| int(i, "round") == n);
    }
    ideas.sort_by(|a, b| points(b).total_cmp(&points(a)).then(int(a, "id").cmp(&int(b, "id"))));
    let top = if c.on("all") { ideas.len() } else { c.val("top").and_then(|n| n.parse().ok()).unwrap_or(TOP_DEFAULT) };
    let rows: Vec<Vec<String>> = ideas
        .iter()
        .take(top)
        .map(|i| {
            let place = if text(i, "place").is_empty() { String::new() } else { format!(" ({})", text(i, "place")) };
            vec![
                int(i, "id").to_string(),
                cut(&format!("{}{place}", text(i, "title")), 70),
                text(i, "lens").into(),
                text(i, "kind").into(),
                points(i).to_string(),
                text(i, "status").into(),
                if text(i, "by") == "wisply" { "wisply".into() } else { "claude".into() },
            ]
        })
        .collect();
    let mut out = table(&["#", "Idea", "Lens", "Kind", "Points", "Status", "By"], &rows);
    if out.is_empty() {
        out = "no ideas".into();
    }
    if ideas.len() > top {
        out.push_str(&format!("\n... +{} more (--all)", ideas.len() - top));
    }
    out
}

fn idea_set(r: &mut Run, c: &Call) -> Result<String, String> {
    let status = c
        .val("status")
        .ok_or("wisply think idea set <ids> --status done|unchecked|dropped|suggestion|new")?
        .to_string();
    if !STATUSES.contains(&status.as_str()) {
        return Err(format!("status is one of {}", STATUSES.join(", ")));
    }
    let ids: Vec<i64> = c.pos.iter().skip(2).filter_map(|s| s.trim_start_matches('#').parse().ok()).collect();
    if ids.is_empty() {
        return Err("which ideas? wisply think idea set 3 4 --status done --proof \"...\"".into());
    }
    for id in &ids {
        let Some(i) = r.ideas().into_iter().find(|i| int(i, "id") == *id) else {
            return Err(format!("idea #{id} does not exist"));
        };
        if status == "done" && c.val("proof").is_none() && text(&i, "proof").is_empty() {
            return Err(format!("#{id}: done only with proof -> --proof \"<command + result>\""));
        }
        if status == "dropped" && c.val("why").is_none() && text(&i, "why").is_empty() {
            return Err(format!("#{id}: dropped only with a reason -> --why \"<reason>\""));
        }
    }
    let round = r.round();
    for id in &ids {
        let i = r.idea_mut(*id).expect("checked above");
        i["status"] = json!(status);
        for k in ["proof", "why", "source"] {
            if let Some(v) = c.val(k) {
                i[k] = json!(v);
            }
        }
        if let Some(g) = c.val("grade").and_then(|g| g.parse::<i64>().ok()) {
            i["grade"] = json!(g.clamp(1, 10));
        }
        i["round_status"] = json!(round);
        if status == "done" || status == "unchecked" {
            safety::mark_after(r, *id);
        }
    }
    r.save()?;
    Ok(format!("{} idea(s) -> {status}", ids.len()))
}

fn round_stats(r: &Run, n: i64) -> (usize, Vec<(&'static str, Vec<Value>)>) {
    let ideas = r.ideas();
    let mine: Vec<Value> = ideas
        .iter()
        .filter(|i| {
            i.get("round_status").and_then(Value::as_i64).unwrap_or(int(i, "round")) == n || int(i, "round") == n
        })
        .cloned()
        .collect();
    let by =
        STATUSES.iter().map(|s| (*s, mine.iter().filter(|i| text(i, "status") == *s).cloned().collect())).collect();
    (ideas.iter().filter(|i| int(i, "round") == n).count(), by)
}

fn of<'a>(by: &'a [(&'static str, Vec<Value>)], st: &str) -> &'a [Value] {
    by.iter().find(|(s, _)| *s == st).map_or(&[], |(_, v)| v.as_slice())
}

fn target(r: &Run, n: i64) -> i64 {
    if n == 1 {
        int(&r.s, "idea_goal")
    } else if text(&r.s, "mode") == "quick" {
        0
    } else {
        sizes(text(&r.s, "size")).1
    }
}

fn gaps(r: &Run, n: i64, by: &[(&'static str, Vec<Value>)]) -> Vec<String> {
    let mut out = Vec::new();
    if r.s.get("criteria").and_then(Value::as_array).is_none_or(Vec::is_empty) {
        out.push("no criteria (step 2: wisply think crit)".into());
    }
    if !r.paths().is_empty() && r.s.pointer("/measures/before").is_none() {
        out.push("no baseline (step 3: wisply think measure before)".into());
    }
    let new = r.ideas().iter().filter(|i| int(i, "round") == n).count() as i64;
    if new < target(r, n) && int(&r.s, "ideas_out") != n {
        out.push(format!(
            "idea goal of round {n} not reached ({new}/{}): keep collecting or say why (idea out)",
            target(r, n)
        ));
    }
    if !of(by, "done").is_empty() && !r.dir.join(format!("diff-r{n}.txt")).exists() {
        out.push("review missing (step 8: wisply think diff, then check it)".into());
    }
    out
}

pub fn round(home: &Home, c: &Call) -> Result<String, String> {
    let mut r = home.load(c.val("run"))?;
    let n = r.round();
    let (new, by) = round_stats(&r, n);
    let missing = gaps(&r, n, &by);
    let anyway = c.val("anyway");
    if !missing.is_empty() && anyway.is_none() {
        let mut lines: Vec<String> = missing.iter().map(|g| format!("LOCKED: {g}")).collect();
        lines.push(
            "round stays open: fill the gaps; only when impossible: wisply think round --anyway \"<reason>\"".into(),
        );
        return Err(lines.join("\n"));
    }
    let mut out = Vec::new();
    if let (false, Some(why)) = (missing.is_empty(), anyway) {
        r.list("skipped").push(json!({"round": n, "gaps": missing, "why": why}));
        out.push(format!("SKIPPED (goes into the report): {}", missing.join("; ")));
    }
    let productive = of(&by, "done").iter().any(|i| int(i, "impact") >= IMPORTANT);
    let open: usize = r.ideas().iter().filter(|i| text(i, "status") == "new" && int(i, "impact") >= IMPORTANT).count();
    let max = int(&r.s, "max_rounds").max(1);
    let (verdict, why) = if text(&r.s, "mode") == "quick" {
        ("STOP", "quick mode".to_string())
    } else if n >= max {
        ("STOP", format!("round {n} is the maximum"))
    } else if productive {
        ("NEXT", "an important improvement landed -> new round on the new state".to_string())
    } else if open > 0 {
        ("NEXT", format!("{open} important ideas still open -> do them or drop them honestly"))
    } else {
        ("STOP", "round without an important improvement".to_string())
    };
    let count = |s: &str| of(&by, s).len();
    r.list("rounds").push(json!({"n": n, "new": new, "done": count("done"), "dropped": count("dropped"),
        "suggestion": count("suggestion"), "unchecked": count("unchecked"), "verdict": verdict, "why": why}));
    if verdict == "NEXT" {
        r.map().insert("round".into(), json!(n + 1));
    }
    r.save()?;
    out.push(format!(
        "round {n}/{max}: {new} new ideas, {} done, {} dropped, {} suggestions{} -> {verdict} ({why})",
        count("done"),
        count("dropped"),
        count("suggestion"),
        if count("unchecked") > 0 { format!(", {} unchecked", count("unchecked")) } else { String::new() }
    ));
    out.push(if verdict == "NEXT" {
        format!("next: step 5 on the new state (wisply think scan again), idea goal round {}: {} (open important ideas first)", n + 1, target(&r, n + 1))
    } else {
        "next: step 10 -> wisply think report".into()
    });
    Ok(out.join("\n"))
}

pub fn report(home: &Home, c: &Call) -> Result<String, String> {
    let mut r = home.load(c.val("run"))?;
    let ideas = r.ideas();
    let by_points = |mut v: Vec<Value>| {
        v.sort_by(|a, b| points(b).total_cmp(&points(a)));
        v
    };
    let pick = |st: &str| ideas.iter().filter(|i| text(i, "status") == st).cloned().collect::<Vec<_>>();
    let done = by_points(pick("done"));
    let dropped = pick("dropped");
    let unchecked = pick("unchecked");
    let open = by_points(
        ideas
            .iter()
            .filter(|i| {
                text(i, "status") == "suggestion" || (text(i, "status") == "new" && int(i, "impact") >= IMPORTANT)
            })
            .cloned()
            .collect(),
    );
    let rounds = r.s.get("rounds").and_then(Value::as_array).map_or(1, |a| a.len().max(1));
    let mut out = vec![format!(
        "**Critical thinking - {}** | {rounds} rounds | {} ideas ({} by wisply) | {} done | {} dropped",
        text(&r.s, "name"),
        ideas.len(),
        ideas.iter().filter(|i| text(i, "by") == "wisply").count(),
        done.len(),
        dropped.len()
    )];
    let rows = |list: &[Value], f: &dyn Fn(&Value) -> String| -> Vec<Vec<String>> {
        list.iter().take(REPORT_ROWS).map(|i| vec![cut(text(i, "title"), 60), cut(&f(i), 70)]).collect()
    };
    if done.is_empty() {
        out.push("No real improvement found - everything was checked anyway (wisply think idea list --all).".into());
    } else {
        out.push(table(
            &["Improved", "Proof"],
            &rows(&done, &|i| {
                let g = i.get("grade").map(|g| format!(" (grade {g})")).unwrap_or_default();
                format!("{}{g}", text(i, "proof"))
            }),
        ));
    }
    if !unchecked.is_empty() {
        out.push(table(
            &["Unchecked (game ran)", "Check with"],
            &rows(&unchecked, &|_| "wisply think measure after".into()),
        ));
    }
    if !dropped.is_empty() {
        out.push(table(&["Dropped", "Reason"], &rows(&dropped, &|i| text(i, "why").to_string())));
    }
    if !open.is_empty() {
        out.push(table(
            &["Open / suggestion", "Why not done"],
            &rows(&open, &|i| {
                let why = text(i, "why");
                if !why.is_empty() {
                    why.to_string()
                } else if text(i, "kind") == "big" {
                    "big - you decide".into()
                } else {
                    "not done in time".into()
                }
            }),
        ));
    }
    let m = r.s.get("measures").cloned().unwrap_or(json!({}));
    if m.get("before").is_some() || m.get("after").is_some() {
        out.push(format!("measured: {}", measure::compare(&m["before"], &m["after"]).replace('\n', " | ")));
    }
    for h in r.s.get("hints").and_then(Value::as_array).cloned().unwrap_or_default() {
        out.push(format!("hint about the goal: {}", h.as_str().unwrap_or("")));
    }
    if let Some(why) = r.s.get("ideas_out_why").and_then(Value::as_object) {
        out.extend(why.iter().map(|(n, w)| format!("ideas ran out (round {n}): {}", w.as_str().unwrap_or(""))));
    }
    for s in r.s.get("skipped").and_then(Value::as_array).cloned().unwrap_or_default() {
        out.push(format!("skipped (round {}): {} - reason: {}", s["round"], s["gaps"], text(&s, "why")));
    }
    for k in r.s.get("criteria_later").and_then(Value::as_array).cloned().unwrap_or_default() {
        out.push(format!("criteria added later: {} - reason: {}", k["criteria"], text(&k, "why")));
    }
    let blind = r.s.get("blind").and_then(Value::as_array).cloned().unwrap_or_default();
    if !blind.is_empty() {
        let won = |w: &str| blind.iter().filter(|b| text(b, "winner") == w).count();
        let open = blind.iter().filter(|b| b.get("winner").is_none()).count();
        out.push(format!(
            "blind choice: new better {} | old better {} | no difference {} | open {open}",
            won("after"),
            won("before"),
            won("same")
        ));
    }
    let small = ideas.iter().filter(|i| text(i, "status") == "new" && int(i, "impact") < IMPORTANT).count();
    let sources = r.s.get("sources").and_then(Value::as_array).map_or(0, Vec::len);
    out.push(format!(
        "sources: {sources}{}",
        if small > 0 { format!(" | {small} small ideas open") } else { String::new() }
    ));
    r.map().insert("status".into(), json!("done"));
    r.map().insert("end".into(), json!(run::now()));
    r.save()?;
    prune(home);
    Ok(out.into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join("\n\n"))
}

fn prune(home: &Home) {
    let finished: Vec<Run> = home.all().into_iter().filter(|r| text(&r.s, "status") == "done").collect();
    let drop = finished.len().saturating_sub(RUNS_KEEP);
    for r in &finished[..drop] {
        let _ = fs::remove_dir_all(&r.dir);
    }
    let kept = &finished[drop..];
    for r in &kept[..kept.len().saturating_sub(BACKUPS_KEEP)] {
        for sub in ["backup", "cp", "blind", "text"] {
            let _ = fs::remove_dir_all(r.dir.join(sub));
        }
    }
}

pub fn status(home: &Home, c: &Call) -> Result<String, String> {
    let r = home.load(c.val("run"))?;
    let ideas = r.ideas();
    let counts: Vec<String> = STATUSES
        .iter()
        .map(|s| (s, ideas.iter().filter(|i| text(i, "status") == *s).count()))
        .filter(|(_, n)| *n > 0)
        .map(|(s, n)| format!("{s} {n}"))
        .collect();
    let crits: Vec<&str> =
        r.s.get("criteria")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
    let measures: Vec<String> =
        r.s.get("measures")
            .and_then(Value::as_object)
            .map(|m| m.iter().map(|(k, v)| format!("{k}: {}", measure::line(v))).collect())
            .unwrap_or_default();
    let mut out = vec![
        format!(
            "run {} | {} | {} | round {}/{} | {}",
            r.id(),
            text(&r.s, "mode"),
            text(&r.s, "size"),
            r.round(),
            int(&r.s, "max_rounds"),
            text(&r.s, "status")
        ),
        format!("goal: {}", text(&r.s, "goal")),
        format!("ideas {}/{}: {}", ideas.len(), int(&r.s, "idea_goal"), counts.join(", ")),
        format!("criteria: {}", if crits.is_empty() { "-".to_string() } else { crits.join(" | ") }),
        format!("measures: {}", if measures.is_empty() { "-".to_string() } else { measures.join(", ") }),
    ];
    let new = ideas.iter().filter(|i| text(i, "status") == "new").count();
    let next = if crits.is_empty() {
        "step 2: wisply think crit \"<criterion>\" ..."
    } else if r.s.pointer("/measures/before").is_none() && !r.paths().is_empty() {
        "step 3: wisply think measure before"
    } else if (ideas.len() as i64) < int(&r.s, "idea_goal") && r.round() == 1 {
        "step 4/5: research + ideas (wisply think scan, wisply think idea add)"
    } else if new > 0 {
        "step 6/7: pick + do (wisply think fix, wisply think idea list)"
    } else {
        "step 8/9: review (wisply think diff), then wisply think round"
    };
    out.push(format!("next: {next}"));
    Ok(out.join("\n"))
}

pub fn runs(home: &Home) -> String {
    let cur = home.current_id().unwrap_or_default();
    let all = home.all();
    let rows: Vec<String> = all
        .iter()
        .rev()
        .take(RUNS_KEEP)
        .rev()
        .map(|r| {
            format!(
                "{} {} | {} | round {} | {} ideas",
                if r.id() == cur { "*" } else { " " },
                r.id(),
                text(&r.s, "status"),
                r.round(),
                r.ideas().len()
            )
        })
        .collect();
    if rows.is_empty() { "no runs yet".into() } else { rows.join("\n") }
}

pub fn brief(env: &Env, home: &Home) -> String {
    let items: Vec<String> = home.lessons().into_iter().take(BRIEF_LESSONS).collect();
    let mut out =
        vec![if items.is_empty() { "lessons: none yet".to_string() } else { "lessons from earlier runs:".to_string() }];
    out.extend(items);
    out.push(if env.playing.is_empty() { "game: none".into() } else { format!("game: {} runs", env.playing) });
    if let Ok(r) = home.load(None)
        && text(&r.s, "status") == "running"
    {
        out.push(format!(
            "open run: {} ({}), round {}/{}, {} ideas -> go on with wisply think status or start a new one",
            r.id(),
            text(&r.s, "name"),
            r.round(),
            int(&r.s, "max_rounds"),
            r.ideas().len()
        ));
    }
    let finished: Vec<Run> = home.all().into_iter().filter(|r| text(&r.s, "status") == "done").collect();
    for r in finished.iter().rev().take(BACKUPS_KEEP) {
        let n = r.ideas().iter().filter(|i| text(i, "status") == "unchecked").count();
        if n > 0 {
            out.push(format!(
                "unchecked from {} ({}): {n} ideas -> without a game: wisply think measure after --run {}",
                r.id(),
                text(&r.s, "name"),
                r.id()
            ));
        }
    }
    out.join("\n")
}

pub fn learn(home: &Home, c: &Call) -> Result<String, String> {
    let body = c.pos[1..].join(" ");
    if c.on("list") || body.trim().is_empty() {
        let items = home.lessons();
        return Ok(if items.is_empty() { "no lessons yet".into() } else { items.join("\n") });
    }
    Ok(run::learn(home, body.trim(), c.val("tag").unwrap_or("")))
}

pub fn packs_line(packs: &BTreeSet<Pack>) -> String {
    packs.iter().map(|p| p.name()).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ideas_parse_with_defaults_and_limits() {
        let v = parse_idea("Cache rules | perf | measure | 3 | 2 | 1 | safe").unwrap();
        assert_eq!((text(&v, "title"), int(&v, "impact"), text(&v, "kind")), ("Cache rules", 3, "safe"));
        let v = parse_idea("Just a title").unwrap();
        assert_eq!((int(&v, "impact"), text(&v, "kind"), text(&v, "by")), (2, "safe", "claude"));
        assert_eq!(parse_idea("x | a | b | 4").unwrap_err(), "impact must be 1, 2 or 3, not '4'");
        assert_eq!(parse_idea("x | a | b | 1 | 1 | 1 | huge").unwrap_err(), "kind must be safe or big, not 'huge'");
        assert_eq!(parse_idea("x | a | b | 1 | 1 | 1 | gross").unwrap()["kind"], "big");
    }
}

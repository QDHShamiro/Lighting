use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::LazyLock;

use regex::Regex;

use super::ask::{Sym, place, syms};
use super::index::Index;

const IMPACT: usize = 6;
const FAMILY: &[(&str, &str)] =
    &[("kotlin", "java"), ("scala", "java"), ("typescript", "javascript"), ("tsx", "javascript"), ("cpp", "c")];

static HUNK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@").expect("hunk"));
static TEST_PATH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(^|/)(tests?|spec|__tests__)/|_test\.|\.test\.|Test\.").expect("tests"));

fn family(lang: &str) -> &str {
    FAMILY.iter().find(|(l, _)| *l == lang).map_or(lang, |(_, f)| f)
}

impl Index {
    pub fn impact(&self, base: Option<&str>, tests: bool) -> Result<String, String> {
        let against = base.unwrap_or("HEAD");
        let ranges = changed_ranges(&self.root, against)?;
        if ranges.is_empty() {
            return Ok(format!("no changes against {against}"));
        }
        let err = |e: rusqlite::Error| e.to_string();
        let mut hit: BTreeMap<i64, Sym> = BTreeMap::new();
        for (path, spans) in &ranges {
            let all = syms(&self.db, "select * from symbols where path = ?1", [path]).map_err(err)?;
            for (a, b) in spans {
                let inside = all.iter().filter(|s| s.line <= *b && s.end >= *a).min_by_key(|s| s.end - s.line);
                if let Some(s) = inside {
                    hit.insert(s.id, s.clone());
                }
            }
        }
        if hit.is_empty() {
            let names: Vec<&String> = ranges.keys().take(8).collect();
            let names: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
            return Ok(format!(
                "{} changed files, no functions or classes touched: {}",
                ranges.len(),
                names.join(", ")
            ));
        }
        let keep = |p: &str| tests || !TEST_PATH.is_match(p);
        let mut q = self.db.prepare("select path, lang from files").map_err(err)?;
        let langs: HashMap<String, String> = q
            .query_map([], |r| Ok((r.get::<_, String>(0)?, family(&r.get::<_, String>(1)?).to_string())))
            .map_err(err)?
            .filter_map(Result::ok)
            .collect();
        let mut lines = vec![format!("changed against {against}: {} symbols in {} files", hit.len(), ranges.len())];
        let mut hits: Vec<&Sym> = hit.values().collect();
        hits.sort_by(|a, b| (&a.path, a.line).cmp(&(&b.path, b.line)));
        let mut refs_q =
            self.db.prepare("select path, line, caller from refs where low = ?1 order by path, line").map_err(err)?;
        for s in hits {
            let lang = langs.get(&s.path);
            let refs: Vec<(String, i64, String)> = refs_q
                .query_map([&s.low], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                    ))
                })
                .map_err(err)?
                .filter_map(Result::ok)
                .filter(|(p, l, _)| keep(p) && langs.get(p) == lang && !(p == &s.path && s.line <= *l && *l <= s.end))
                .collect();
            let head = place(s);
            if refs.is_empty() {
                lines.push(format!("{head} <- no callers found"));
                continue;
            }
            let shown: Vec<String> = refs
                .iter()
                .take(IMPACT)
                .map(|(p, l, c)| format!("{p}:{l}{}", if c.is_empty() { String::new() } else { format!(" {c}") }))
                .collect();
            let more = if refs.len() > IMPACT { ", ..." } else { "" };
            lines.push(format!("{head} <- {}: {}{more}", refs.len(), shown.join(", ")));
            let names: BTreeSet<String> =
                refs.iter().filter(|(_, _, c)| !c.is_empty()).map(|(_, _, c)| c.to_lowercase()).collect();
            let mut second: BTreeSet<String> = BTreeSet::new();
            for n in &names {
                let rows = refs_q
                    .query_map([n], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(2)?.unwrap_or_default()))
                    })
                    .map_err(err)?;
                second.extend(
                    rows.filter_map(Result::ok)
                        .filter(|(p, c)| !c.is_empty() && keep(p) && langs.get(p) == lang)
                        .map(|(_, c)| c),
                );
            }
            if !second.is_empty() {
                let list: Vec<&String> = second.iter().take(IMPACT).collect();
                let list: Vec<&str> = list.iter().map(|s| s.as_str()).collect();
                let more = if second.len() > IMPACT { ", ..." } else { "" };
                lines.push(format!("      then: {}{more}", list.join(", ")));
            }
        }
        Ok(lines.join("\n"))
    }
}

fn changed_ranges(root: &Path, base: &str) -> Result<BTreeMap<String, Vec<(i64, i64)>>, String> {
    let mut cmd = Command::new("git");
    cmd.args(["-c", "core.quotePath=false", "-C"]).arg(root).args(["diff", "--relative", "-U0", base, "--"]);
    cmd.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let out = cmd.output().map_err(|e| format!("git diff failed: {e}"))?;
    if !out.status.success() {
        let why: String = String::from_utf8_lossy(&out.stderr).trim().chars().take(200).collect();
        return Err(format!("git diff failed: {why}"));
    }
    let mut ranges: BTreeMap<String, Vec<(i64, i64)>> = BTreeMap::new();
    let mut path: Option<String> = None;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(p) = line.strip_prefix("+++ ") {
            path = if p == "/dev/null" { None } else { Some(p.strip_prefix("b/").unwrap_or(p).to_string()) };
        }
        if let (Some(m), Some(p)) = (HUNK.captures(line), &path) {
            let start: i64 = m[1].parse().unwrap_or(0);
            let n: i64 = m.get(2).map_or(1, |g| g.as_str().parse().unwrap_or(1));
            ranges.entry(p.clone()).or_default().push((start, start + n.max(1) - 1));
        }
    }
    Ok(ranges)
}

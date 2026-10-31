use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::files::{self, Pack};
use crate::measure::{quiet, run};
use crate::rules::{Finding, drop_repeats, recompress};

const TOOL_WAIT: Duration = Duration::from_secs(300);

pub struct Plan {
    pub fixer: &'static str,
    pub title: String,
    pub files: Vec<PathBuf>,
}

fn works(program: &str, arg: &str) -> bool {
    quiet(&mut Command::new(program)).arg(arg).output().is_ok_and(|o| o.status.success())
}

fn prettier(root: &Path) -> Option<PathBuf> {
    let bin = root.join("node_modules").join(".bin");
    [bin.join("prettier.cmd"), bin.join("prettier")].into_iter().find(|p| p.exists())
}

fn listed(out: &str, root: &Path, prefix: &str) -> Vec<PathBuf> {
    out.lines()
        .filter_map(|l| {
            let l = l.trim();
            let l = l.strip_prefix(prefix).unwrap_or(if prefix.is_empty() { l } else { "" }).trim();
            (!l.is_empty()).then(|| root.join(l))
        })
        .filter(|p| p.is_file())
        .collect()
}

pub fn plans(root: &Path, found: &[Finding], packs: &std::collections::BTreeSet<Pack>, deep: bool) -> Vec<Plan> {
    let mut by: BTreeMap<&'static str, Vec<PathBuf>> = BTreeMap::new();
    for f in found {
        if let Some(fixer) = f.fixer {
            let p = root.join(&f.file);
            let list = by.entry(fixer).or_default();
            if !list.contains(&p) {
                list.push(p);
            }
        }
    }
    let mut out: Vec<Plan> = by
        .into_iter()
        .map(|(fixer, files)| {
            let what = match fixer {
                "png" => "recompress PNGs without loss",
                "repeat" => "drop repeated words",
                "spaces" => "strip trailing spaces",
                _ => fixer,
            };
            Plan { fixer, title: format!("Wisply: {what} ({} files)", files.len()), files }
        })
        .collect();
    let rust = packs.contains(&Pack::Rust) && root.join("Cargo.toml").exists();
    if rust && works("cargo", "--version") {
        let check = run("cargo fmt --check -- -l", root, TOOL_WAIT);
        let list: Vec<PathBuf> = check.out.lines().map(|l| PathBuf::from(l.trim())).filter(|p| p.is_file()).collect();
        if check.code != 0 && !list.is_empty() {
            out.push(Plan {
                fixer: "rustfmt",
                title: format!("Wisply: cargo fmt ({} files)", list.len()),
                files: list,
            });
        }
        if deep {
            let all: Vec<PathBuf> =
                files::walk(&[root.to_path_buf()]).into_iter().filter(|p| files::ext(p) == "rs").collect();
            out.push(Plan {
                fixer: "clippy",
                title: "Wisply: cargo clippy --fix (machine-applicable)".into(),
                files: all,
            });
        }
    }
    if let Some(bin) = prettier(root).filter(|_| packs.contains(&Pack::Web)) {
        let check = run(&format!("\"{}\" --list-different .", bin.display()), root, TOOL_WAIT);
        let list = listed(&check.out, root, "");
        if !list.is_empty() {
            out.push(Plan {
                fixer: "prettier",
                title: format!("Wisply: prettier ({} files)", list.len()),
                files: list,
            });
        }
    }
    if packs.contains(&Pack::Python) && works("ruff", "--version") {
        let check = run("ruff format --check .", root, TOOL_WAIT);
        let list = listed(&check.out, root, "Would reformat:");
        if !list.is_empty() {
            out.push(Plan { fixer: "ruff", title: format!("Wisply: ruff format ({} files)", list.len()), files: list });
        }
    }
    out
}

fn rewrite(p: &Path, f: impl Fn(&str) -> String) -> Result<bool, String> {
    let Some(text) = files::read(p) else { return Ok(false) };
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let end = text.ends_with('\n');
    let lines: Vec<String> = text.lines().map(&f).collect();
    let mut new = lines.join(nl);
    if end {
        new.push_str(nl);
    }
    if new == text {
        return Ok(false);
    }
    fs::write(p, new).map_err(|e| format!("cannot write {}: {e}", p.display()))?;
    Ok(true)
}

fn repeat_lines(text: &str) -> String {
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut code = false;
    let mut out: Vec<String> = Vec::new();
    for l in text.lines() {
        if l.trim_start().starts_with("```") {
            code = !code;
        }
        out.push(if code || l.contains('`') { l.to_string() } else { drop_repeats(l) });
    }
    let mut s = out.join(nl);
    if text.ends_with('\n') {
        s.push_str(nl);
    }
    s
}

pub fn apply(plan: &Plan, root: &Path) -> Result<String, String> {
    match plan.fixer {
        "spaces" => {
            let mut n = 0;
            for p in &plan.files {
                n += usize::from(rewrite(p, |l| l.trim_end_matches([' ', '\t']).to_string())?);
            }
            Ok(format!("{n} files without trailing spaces"))
        }
        "repeat" => {
            let mut n = 0;
            for p in &plan.files {
                let Some(text) = files::read(p) else { continue };
                let new = repeat_lines(&text);
                if new != text {
                    fs::write(p, new).map_err(|e| e.to_string())?;
                    n += 1;
                }
            }
            Ok(format!("{n} files without repeated words"))
        }
        "png" => {
            let (mut before, mut after) = (0u64, 0u64);
            for p in &plan.files {
                let raw = fs::read(p).map_err(|e| e.to_string())?;
                let Some(small) = recompress(&raw) else { continue };
                let same = image::load_from_memory(&raw).ok().map(|i| i.to_rgba8().into_raw())
                    == image::load_from_memory(&small).ok().map(|i| i.to_rgba8().into_raw());
                if !same {
                    continue;
                }
                fs::write(p, &small).map_err(|e| e.to_string())?;
                before += raw.len() as u64;
                after += small.len() as u64;
            }
            Ok(format!("pixels identical, {} KB -> {} KB", before.div_ceil(1024), after.div_ceil(1024)))
        }
        "rustfmt" => tool("cargo fmt", root),
        "clippy" => tool("cargo clippy --fix --allow-dirty --allow-staged --allow-no-vcs -q", root),
        "prettier" => {
            let bin = prettier(root).ok_or("prettier is gone")?;
            let list: Vec<String> = plan.files.iter().map(|p| format!("\"{}\"", p.display())).collect();
            tool(&format!("\"{}\" --write {}", bin.display(), list.join(" ")), root)
        }
        "ruff" => {
            let list: Vec<String> = plan.files.iter().map(|p| format!("\"{}\"", p.display())).collect();
            tool(&format!("ruff format {}", list.join(" ")), root)
        }
        other => Err(format!("no fixer called {other}")),
    }
}

fn tool(cmd: &str, root: &Path) -> Result<String, String> {
    let r = run(cmd, root, TOOL_WAIT);
    if r.code == 0 {
        Ok(format!("{cmd} ok ({} s)", r.secs))
    } else {
        Err(format!("{cmd} failed (exit {}): {}", r.code, r.out.lines().last().unwrap_or("")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_fixers_keep_line_endings_and_code() {
        let dir = std::env::temp_dir().join(format!("wisply-think-fix-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.yml");
        fs::write(&a, "a: 1  \r\nb: 2\t\r\n").unwrap();
        let md = dir.join("r.md");
        fs::write(&md, "This is is fine.\n```\nthe the\n```\nkeep `x x` here\n").unwrap();
        let plan = Plan { fixer: "spaces", title: String::new(), files: vec![a.clone()] };
        assert_eq!(apply(&plan, &dir).unwrap(), "1 files without trailing spaces");
        assert_eq!(fs::read_to_string(&a).unwrap(), "a: 1\r\nb: 2\r\n");
        let plan = Plan { fixer: "repeat", title: String::new(), files: vec![md.clone()] };
        apply(&plan, &dir).unwrap();
        assert_eq!(fs::read_to_string(&md).unwrap(), "This is fine.\n```\nthe the\n```\nkeep `x x` here\n");
        let _ = fs::remove_dir_all(&dir);
    }
}

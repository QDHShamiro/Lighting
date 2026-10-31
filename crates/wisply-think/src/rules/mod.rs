mod code;
mod docs;
mod image;
mod plugin;
mod web;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::files::{self, Pack};
use crate::personal::{self, Rule};

pub use code::{Function, functions};
pub use docs::drop_repeats;
pub use image::{png_smaller, recompress};
pub use plugin::{entries, unquote};

const IMAGE_BUDGET: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub rule: String,
    pub pack: Pack,
    pub file: String,
    pub line: usize,
    pub why: String,
    pub fix: String,
    pub impact: u8,
    pub sure: u8,
    pub effort: u8,
    pub fixer: Option<&'static str>,
}

impl Finding {
    pub fn new(rule: &str, pack: Pack, line: usize, why: impl Into<String>, fix: impl Into<String>) -> Finding {
        Finding {
            rule: rule.to_string(),
            pack,
            file: String::new(),
            line,
            why: why.into(),
            fix: fix.into(),
            impact: 2,
            sure: 2,
            effort: 2,
            fixer: None,
        }
    }

    pub fn rate(mut self, impact: u8, sure: u8, effort: u8) -> Finding {
        self.impact = impact;
        self.sure = sure;
        self.effort = effort;
        self
    }

    pub fn fixed_by(mut self, fixer: &'static str) -> Finding {
        self.fixer = Some(fixer);
        self
    }

    pub fn place(&self) -> String {
        if self.line > 0 { format!("{}:{}", self.file, self.line) } else { self.file.clone() }
    }

    pub fn key(&self) -> String {
        format!("{}|{}|{}", self.rule, self.file, self.why)
    }

    pub fn points(&self) -> f64 {
        f64::from(self.impact) * f64::from(self.sure) / f64::from(self.effort.max(1))
    }
}

pub struct Scope<'a> {
    pub root: &'a Path,
    pub packs: &'a BTreeSet<Pack>,
    pub rules: &'a [Rule],
}

pub fn file(scope: &Scope, path: &Path, deep: bool) -> Vec<Finding> {
    let rel = files::rel(scope.root, path);
    let mut out = Vec::new();
    let e = files::ext(path);
    let on = |p: Pack| scope.packs.contains(&p);
    if files::has_ext(path, files::IMAGE_EXT) && on(Pack::Images) {
        out.extend(image::check(path, &e, deep));
    }
    if files::has_ext(path, files::TEXT_EXT)
        && let Some(text) = files::read(path)
    {
        let test = files::is_test(path);
        if files::has_ext(path, files::CODE_EXT) && on(Pack::Code) {
            out.extend(code::check(&e, &text, test, scope.packs));
        }
        if on(Pack::Web) {
            out.extend(web::check(&e, &text));
        }
        if e == "md" && on(Pack::Docs) {
            out.extend(docs::check(path, scope.root, &text));
        }
        if on(Pack::JavaPlugin) || on(Pack::McGui) {
            out.extend(plugin::check(path, &text, scope.packs));
        }
        out.extend(personal::check(scope.rules, path, &e, &text, test, scope.packs));
        if !matches!(e.as_str(), "md" | "rs" | "svg") && text.lines().any(|l| l.ends_with(' ') || l.ends_with('\t')) {
            let n = text.lines().filter(|l| l.ends_with(' ') || l.ends_with('\t')).count();
            out.push(
                Finding::new("trailing-spaces", Pack::Code, 0, format!("{n} lines end with spaces"), "strip them")
                    .rate(1, 3, 1)
                    .fixed_by("spaces"),
            );
        }
    }
    for f in &mut out {
        f.file = rel.clone();
    }
    out
}

pub fn scan(scope: &Scope, list: &[PathBuf]) -> Vec<Finding> {
    let started = Instant::now();
    let mut out = Vec::new();
    for path in list {
        let deep = started.elapsed() < IMAGE_BUDGET;
        out.extend(file(scope, path, deep));
    }
    if scope.packs.contains(&Pack::Code) {
        let code: Vec<(String, String)> = list
            .iter()
            .filter(|p| files::has_ext(p, files::CODE_EXT) && !files::is_test(p))
            .filter_map(|p| files::read(p).map(|t| (files::rel(scope.root, p), t)))
            .collect();
        out.extend(code::duplicates(&code));
    }
    out.sort_by(|a, b| b.points().total_cmp(&a.points()).then(a.file.cmp(&b.file)).then(a.line.cmp(&b.line)));
    out
}

pub fn line_of(text: &str, at: usize) -> usize {
    text[..at.min(text.len())].bytes().filter(|b| *b == b'\n').count() + 1
}

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use super::Finding;
use crate::files::{self, Pack};

const DRIVERS: &[&str] = &["sqlite-jdbc", "HikariCP", "mysql-connector-j"];

static DEP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<dependency>(.*?)</dependency>").expect("dep pattern"));
static SLOT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*slots?\s*:\s*\[?([0-9][0-9,\s-]*)\]?\s*$").expect("slot pattern"));
static SIZE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*(size|rows)\s*:\s*(\d+)\s*$").expect("size pattern"));

pub struct Entry<'a> {
    pub line: usize,
    pub indent: usize,
    pub key: &'a str,
    pub value: &'a str,
}

pub fn entries(text: &str) -> Vec<Entry<'_>> {
    text.lines()
        .enumerate()
        .filter_map(|(i, l)| {
            let t = l.trim_start();
            if t.starts_with('#') || t.is_empty() {
                return None;
            }
            let indent = l.len() - t.len();
            if let Some(item) = t.strip_prefix("- ") {
                return Some(Entry { line: i + 1, indent, key: "-", value: item.trim() });
            }
            let (key, value) = t.split_once(':')?;
            if key.contains(' ') && !key.starts_with(['"', '\'']) {
                return None;
            }
            Some(Entry { line: i + 1, indent, key: key.trim().trim_matches(['"', '\'']), value: value.trim() })
        })
        .collect()
}

pub fn unquote(v: &str) -> &str {
    v.trim().trim_matches(|c| c == '"' || c == '\'')
}

fn slots(spec: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part.split_once('-') {
            Some((a, b)) => {
                if let (Ok(a), Ok(b)) = (a.trim().parse::<u32>(), b.trim().parse::<u32>()) {
                    out.extend(a..=b.min(a + 54));
                }
            }
            None => out.extend(part.parse::<u32>().ok()),
        }
    }
    out
}

fn gui(text: &str, out: &mut Vec<Finding>) {
    let size = SIZE.captures(text).and_then(|c| {
        let n: u32 = c[2].parse().ok()?;
        Some(if &c[1] == "rows" { n * 9 } else { n })
    });
    let mut seen: HashMap<u32, usize> = HashMap::new();
    let mut double = None;
    let mut outside = None;
    for c in SLOT.captures_iter(text) {
        let at = super::line_of(text, c.get(0).map_or(0, |m| m.start()));
        for s in slots(&c[1]) {
            if let Some(first) = seen.insert(s, at)
                && first != at
                && double.is_none()
            {
                double = Some((at, s, first));
            }
            if size.is_some_and(|n| s >= n) && outside.is_none() {
                outside = Some((at, s));
            }
        }
    }
    if let Some((at, s, first)) = double {
        out.push(
            Finding::new(
                "slot-twice",
                Pack::McGui,
                at,
                format!("slot {s} is used twice (first at line {first})"),
                "give each item its own slot",
            )
            .rate(2, 3, 1),
        );
    }
    if let (Some((at, s)), Some(n)) = (outside, size) {
        out.push(
            Finding::new(
                "slot-outside",
                Pack::McGui,
                at,
                format!("slot {s} is outside the {n}-slot GUI"),
                format!("use 0-{}", n - 1),
            )
            .rate(2, 3, 1),
        );
    }
}

pub fn check(path: &Path, text: &str, packs: &BTreeSet<Pack>) -> Vec<Finding> {
    let mut out = Vec::new();
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let low = files::slashed(path).to_lowercase();
    if packs.contains(&Pack::JavaPlugin) && (name == "plugin.yml" || name == "paper-plugin.yml") {
        let folia = entries(text).iter().any(|e| e.key == "folia-supported" && unquote(e.value) == "true");
        if !folia {
            out.push(
                Finding::new(
                    "folia-flag",
                    Pack::JavaPlugin,
                    0,
                    "plugin.yml does not say folia-supported: true",
                    "add folia-supported: true once it runs on Folia",
                )
                .rate(2, 2, 1),
            );
        }
    }
    if packs.contains(&Pack::JavaPlugin) && name == "pom.xml" {
        for c in DEP.captures_iter(text) {
            let body = &c[1];
            let Some(driver) = DRIVERS.iter().find(|d| body.contains(&format!("<artifactId>{d}</artifactId>"))) else {
                continue;
            };
            if !body.contains("<scope>provided</scope>") {
                let at = super::line_of(text, c.get(0).map_or(0, |m| m.start()));
                out.push(
                    Finding::new(
                        "bundled-driver",
                        Pack::JavaPlugin,
                        at,
                        format!("{driver} is packed into the jar"),
                        "scope provided + list it under libraries in plugin.yml",
                    )
                    .rate(2, 3, 1),
                );
            }
        }
    }
    if packs.contains(&Pack::McGui)
        && files::has_ext(path, &["yml", "yaml"])
        && (low.contains("/gui/") || low.contains("/guis/"))
    {
        gui(text, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_files_and_guis_are_checked() {
        let packs: BTreeSet<Pack> = [Pack::JavaPlugin, Pack::McGui].into_iter().collect();
        let got = check(Path::new("res/plugin.yml"), "name: X\nmain: a.B\n", &packs);
        assert_eq!(got[0].rule, "folia-flag");
        assert!(check(Path::new("res/plugin.yml"), "name: X\nfolia-supported: true\n", &packs).is_empty());
        let pom = "<dependency><artifactId>HikariCP</artifactId></dependency>\n<dependency><artifactId>sqlite-jdbc</artifactId><scope>provided</scope></dependency>";
        let got = check(Path::new("pom.xml"), pom, &packs);
        assert_eq!(got.len(), 1);
        assert!(got[0].why.starts_with("HikariCP"));
        let shop =
            "title: Shop\nrows: 3\nitems:\n  a:\n    slot: 4\n  b:\n    slot: 4\n  fill:\n    slots: [0-3, 27]\n";
        let got: Vec<String> =
            check(Path::new("plugins/X/gui/shop.yml"), shop, &packs).into_iter().map(|f| f.why).collect();
        assert_eq!(got, ["slot 4 is used twice (first at line 5)", "slot 27 is outside the 27-slot GUI"]);
        let e = entries("a:\n  b: \"x\"\n  - item\n");
        assert_eq!((e[1].key, unquote(e[1].value), e[2].key, e[2].value), ("b", "x", "-", "item"));
    }
}

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde_json::{Value, json};

use super::scan::Finding;

const OSV_BATCH: &str = "https://api.osv.dev/v1/querybatch";
const OSV_VULN: &str = "https://api.osv.dev/v1/vulns/";
const OSV_WAIT: Duration = Duration::from_secs(30);
const BATCH: usize = 900;
const DETAILS: usize = 30;
const SUMMARY_CHARS: usize = 90;

static AGENT: LazyLock<ureq::Agent> =
    LazyLock::new(|| ureq::Agent::config_builder().http_status_as_error(false).build().into());
static MAVEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<dependency>(.*?)</dependency>").expect("maven"));
static TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<(groupId|artifactId|version)>\s*([^<\s]+)\s*</").expect("tag"));
static PIP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^\s*([A-Za-z0-9_.\-\[\]]+)\s*==\s*([^\s;#]+)").expect("pip"));

#[derive(Debug, Clone, PartialEq)]
pub struct Dep {
    pub file: String,
    pub ecosystem: &'static str,
    pub name: String,
    pub version: String,
}

pub fn cargo_lock(file: &str, text: &str) -> Vec<Dep> {
    let mut out = Vec::new();
    for block in text.split("[[package]]").skip(1) {
        let field = |key: &str| {
            block.lines().find_map(|l| {
                l.trim()
                    .strip_prefix(key)
                    .and_then(|r| r.trim().strip_prefix('='))
                    .map(|v| v.trim().trim_matches('"').to_string())
            })
        };
        if let (Some(name), Some(version), Some(_)) = (field("name"), field("version"), field("source")) {
            out.push(Dep { file: file.into(), ecosystem: "crates.io", name, version });
        }
    }
    out
}

pub fn npm_lock(file: &str, text: &str) -> Vec<Dep> {
    let Ok(data) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    let Some(packages) = data["packages"].as_object() else { return Vec::new() };
    packages
        .iter()
        .filter_map(|(path, info)| {
            let name =
                path.rsplit("node_modules/").next().filter(|n| !n.is_empty() && path.contains("node_modules/"))?;
            let version = info["version"].as_str()?;
            Some(Dep { file: file.into(), ecosystem: "npm", name: name.into(), version: version.into() })
        })
        .collect()
}

pub fn requirements(file: &str, text: &str) -> Vec<Dep> {
    PIP.captures_iter(text)
        .map(|m| Dep {
            file: file.into(),
            ecosystem: "PyPI",
            name: m[1].split('[').next().unwrap_or("").into(),
            version: m[2].into(),
        })
        .collect()
}

pub fn pom(file: &str, text: &str) -> Vec<Dep> {
    MAVEN
        .captures_iter(text)
        .filter_map(|block| {
            let mut tags: BTreeMap<&str, &str> = BTreeMap::new();
            for t in TAG.captures_iter(block.get(1)?.as_str()) {
                if let (Some(k), Some(v)) = (t.get(1), t.get(2)) {
                    tags.entry(k.as_str()).or_insert(v.as_str());
                }
            }
            let (group, artifact, version) = (tags.get("groupId")?, tags.get("artifactId")?, tags.get("version")?);
            (!version.contains("${")).then(|| Dep {
                file: file.into(),
                ecosystem: "Maven",
                name: format!("{group}:{artifact}"),
                version: version.to_string(),
            })
        })
        .collect()
}

pub fn collect(root: &Path, names: &[String]) -> (Vec<Dep>, Vec<String>) {
    let mut deps = Vec::new();
    let mut notes = Vec::new();
    for name in names {
        let file = name.rsplit('/').next().unwrap_or(name);
        let read = || fs::read_to_string(root.join(name)).unwrap_or_default();
        match file {
            "Cargo.lock" => deps.extend(cargo_lock(name, &read())),
            "package-lock.json" => deps.extend(npm_lock(name, &read())),
            "pom.xml" => deps.extend(pom(name, &read())),
            f if f.starts_with("requirements") && f.ends_with(".txt") => deps.extend(requirements(name, &read())),
            "package.json" if !names.iter().any(|n| n.ends_with("package-lock.json")) => {
                notes.push(format!("{name}: no package-lock.json, exact versions unknown (npm install writes one)"));
            }
            _ => {}
        }
    }
    deps.sort_by(|a, b| (&a.ecosystem, &a.name, &a.version).cmp(&(&b.ecosystem, &b.name, &b.version)));
    deps.dedup_by(|a, b| a.ecosystem == b.ecosystem && a.name == b.name && a.version == b.version);
    (deps, notes)
}

fn post(body: &Value) -> Result<Value, String> {
    let mut resp = AGENT
        .post(OSV_BATCH)
        .config()
        .timeout_global(Some(OSV_WAIT))
        .build()
        .header("Content-Type", "application/json")
        .send(&body.to_string())
        .map_err(|e| format!("osv.dev not reachable ({e})"))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().with_config().limit(u64::MAX).read_to_string().unwrap_or_default();
    if status != 200 {
        return Err(format!("osv.dev answered {status}"));
    }
    serde_json::from_str(&text).map_err(|_| "osv.dev sent no JSON".into())
}

fn details(id: &str) -> Option<Value> {
    let mut resp = AGENT.get(format!("{OSV_VULN}{id}")).config().timeout_global(Some(OSV_WAIT)).build().call().ok()?;
    serde_json::from_str(&resp.body_mut().read_to_string().ok()?).ok()
}

fn fixed_in(vuln: &Value, dep: &Dep) -> Option<String> {
    let now = version_key(&dep.version);
    vuln["affected"]
        .as_array()?
        .iter()
        .filter(|a| a["package"]["name"].as_str() == Some(dep.name.as_str()))
        .flat_map(|a| a["ranges"].as_array().cloned().unwrap_or_default())
        .filter(|r| r["type"].as_str() != Some("GIT"))
        .flat_map(|r| r["events"].as_array().cloned().unwrap_or_default())
        .filter_map(|e| e["fixed"].as_str().map(String::from))
        .filter(|v| version_key(v) > now)
        .min_by_key(|v| version_key(v))
}

pub fn version_key(v: &str) -> Vec<u64> {
    v.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty()).map(|p| p.parse().unwrap_or(0)).collect()
}

fn line_of(root: &Path, dep: &Dep) -> usize {
    let text = fs::read_to_string(root.join(&dep.file)).unwrap_or_default();
    let short = dep.name.rsplit(':').next().unwrap_or(&dep.name);
    text.lines().position(|l| l.contains(short)).map_or(1, |i| i + 1)
}

pub fn check(root: &Path, deps: &[Dep]) -> Result<Vec<Finding>, String> {
    let mut hits: Vec<(usize, Vec<String>)> = Vec::new();
    for (chunk_at, chunk) in deps.chunks(BATCH).enumerate() {
        let queries: Vec<Value> = chunk
            .iter()
            .map(|d| json!({"package": {"name": d.name, "ecosystem": d.ecosystem}, "version": d.version}))
            .collect();
        let data = post(&json!({"queries": queries}))?;
        for (i, result) in data["results"].as_array().into_iter().flatten().enumerate() {
            let ids: Vec<String> = result["vulns"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v["id"].as_str().map(String::from))
                .collect();
            if !ids.is_empty() {
                hits.push((chunk_at * BATCH + i, ids));
            }
        }
    }
    let mut known: BTreeMap<String, Option<Value>> = BTreeMap::new();
    let mut out = Vec::new();
    for (at, ids) in hits {
        let Some(dep) = deps.get(at) else { continue };
        let mut fixes = Vec::new();
        let mut about = String::new();
        for id in &ids {
            if known.len() < DETAILS && !known.contains_key(id) {
                known.insert(id.clone(), details(id));
            }
            if let Some(Some(v)) = known.get(id) {
                fixes.extend(fixed_in(v, dep));
                if about.is_empty() {
                    about = v["summary"].as_str().unwrap_or("").chars().take(SUMMARY_CHARS).collect();
                }
            }
        }
        let shown: Vec<&str> = ids.iter().take(3).map(String::as_str).collect();
        let more = if ids.len() > 3 { format!(" +{}", ids.len() - 3) } else { String::new() };
        let about = if about.is_empty() { String::new() } else { format!(": {about}") };
        fixes.sort_by_key(|v| version_key(v));
        let fix = fixes.last().map_or("no fixed version listed yet, check the advisory".to_string(), |v| {
            format!("update to {v} or newer")
        });
        out.push(Finding {
            file: dep.file.clone(),
            line: line_of(root, dep),
            why: format!("{} {} has {}{more}{about}", dep.name, dep.version, shown.join(", ")),
            fix,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_files_and_manifests_give_exact_versions() {
        let lock = "[[package]]\nname = \"serde\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n\n[[package]]\nname = \"mine\"\nversion = \"0.1.0\"\n";
        assert_eq!(
            cargo_lock("Cargo.lock", lock),
            vec![Dep {
                file: "Cargo.lock".into(),
                ecosystem: "crates.io",
                name: "serde".into(),
                version: "1.0.0".into()
            }]
        );
        let npm = r#"{"packages": {"": {"version": "1.0.0"}, "node_modules/lodash": {"version": "4.17.20"}, "node_modules/a/node_modules/@x/y": {"version": "2.0.0"}}}"#;
        let names: Vec<String> =
            npm_lock("package-lock.json", npm).into_iter().map(|d| format!("{}@{}", d.name, d.version)).collect();
        assert_eq!(names, vec!["lodash@4.17.20", "@x/y@2.0.0"]);
        let req = requirements(
            "requirements.txt",
            "requests==2.19.0\n# x==1\nflask[async] == 2.0.1 ; python_version>'3'\nloose>=1\n",
        );
        let names: Vec<String> = req.into_iter().map(|d| format!("{}=={}", d.name, d.version)).collect();
        assert_eq!(names, vec!["requests==2.19.0", "flask==2.0.1"]);
        let pom_text = "<dependency><groupId>org.yaml</groupId><artifactId>snakeyaml</artifactId><version>1.30</version></dependency>\n<dependency><groupId>a</groupId><artifactId>b</artifactId><version>${v}</version></dependency>";
        let mut fixes = vec!["2.9.0", "2.10.1", "2.10.0"];
        fixes.sort_by_key(|v| version_key(v));
        assert_eq!(fixes, vec!["2.9.0", "2.10.0", "2.10.1"]);
        let vuln = json!({"affected": [{"package": {"name": "requests"}, "ranges": [
            {"type": "GIT", "events": [{"introduced": "0"}, {"fixed": "74ea7cf7a6a27a4eeb2ae24e162bcc942a6706d5"}]},
            {"type": "ECOSYSTEM", "events": [{"introduced": "0"}, {"fixed": "1.9.0"}, {"introduced": "2.0"}, {"fixed": "2.20.0"}]}
        ]}]});
        let dep = Dep { file: "r.txt".into(), ecosystem: "PyPI", name: "requests".into(), version: "2.19.0".into() };
        assert_eq!(fixed_in(&vuln, &dep).as_deref(), Some("2.20.0"));
        let deps = pom("pom.xml", pom_text);
        assert_eq!(deps.len(), 1);
        assert_eq!((deps[0].name.as_str(), deps[0].version.as_str()), ("org.yaml:snakeyaml", "1.30"));
    }
}

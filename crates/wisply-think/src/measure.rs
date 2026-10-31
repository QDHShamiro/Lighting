use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::thread;
use std::time::{Duration, Instant};

use regex::Regex;
use serde_json::{Value, json};

use crate::files;
use crate::rules::functions;

pub const TIMEOUT: Duration = Duration::from_secs(900);
const TAIL_LINES: usize = 12;
const CCN_WARN: usize = 10;

pub fn kind_of(root: &Path) -> &'static str {
    let at = |m: &str| root.join(m).exists();
    if at("SKILL.md") {
        "skill"
    } else if at("Cargo.toml") {
        "cargo"
    } else if at("pom.xml") {
        "maven"
    } else if at("build.gradle") || at("build.gradle.kts") {
        "gradle"
    } else if at("package.json") {
        "npm"
    } else if at("pyproject.toml") || at("setup.py") || at("requirements.txt") {
        "python"
    } else {
        ""
    }
}

pub fn commands(kind: &str, root: &Path) -> (Option<String>, Option<String>) {
    let gradle = if cfg!(windows) { "gradlew.bat" } else { "./gradlew" };
    let gradle =
        if root.join(if cfg!(windows) { "gradlew.bat" } else { "gradlew" }).exists() { gradle } else { "gradle" };
    let pair = |b: &str, t: &str| (Some(b.to_string()), Some(t.to_string()));
    match kind {
        "cargo" => pair("cargo build -q", "cargo test -q"),
        "maven" => pair("mvn -q -B compile", "mvn -q -B test"),
        "gradle" => pair(&format!("{gradle} build -x test"), &format!("{gradle} test")),
        "npm" => pair("npm run build --if-present", "npm test --if-present"),
        "python" => pair("python -m compileall -q .", "python -m pytest -q"),
        "skill" => (Some("lint-skill".into()), Some("selftest".into())),
        _ => (None, None),
    }
}

pub fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

pub struct Ran {
    pub code: i32,
    pub secs: f64,
    pub out: String,
}

pub fn run(cmd: &str, cwd: &Path, limit: Duration) -> Ran {
    let started = Instant::now();
    let mut c = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(cmd);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(cmd);
        c
    };
    let child =
        quiet(&mut c).current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
    let mut child = match child {
        Ok(ch) => ch,
        Err(e) => return Ran { code: 127, secs: 0.0, out: format!("cannot start {cmd}: {e}") },
    };
    let pipe = |r: Option<Box<dyn Read + Send>>| {
        thread::spawn(move || {
            let mut s = String::new();
            if let Some(mut r) = r {
                let mut raw = Vec::new();
                let _ = r.read_to_end(&mut raw);
                s = String::from_utf8_lossy(&raw).to_string();
            }
            s
        })
    };
    let out = pipe(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = pipe(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(1),
            Ok(None) if started.elapsed() > limit => {
                let _ = child.kill();
                let _ = child.wait();
                break 124;
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(_) => break 1,
        }
    };
    let mut text = out.join().unwrap_or_default();
    text.push_str(&err.join().unwrap_or_default());
    if code == 124 {
        text.push_str(&format!("\ntimed out after {} s", limit.as_secs()));
    }
    Ran { code, secs: (started.elapsed().as_secs_f64() * 100.0).round() / 100.0, out: text }
}

fn re(p: &str) -> Regex {
    Regex::new(p).expect("test count pattern")
}

static CARGO: LazyLock<Regex> = LazyLock::new(|| re(r"test result: \w+\. (\d+) passed; (\d+) failed"));
static NEXTEST: LazyLock<Regex> = LazyLock::new(|| re(r"(\d+) tests? run: (\d+) passed(?:.*?(\d+) failed)?"));
static PYTEST: LazyLock<Regex> = LazyLock::new(|| re(r"(?:(\d+) failed, )?(\d+) passed"));
static JEST: LazyLock<Regex> = LazyLock::new(|| re(r"Tests:\s+(?:(\d+) failed, )?(\d+) passed, (\d+) total"));
static MAVEN: LazyLock<Regex> =
    LazyLock::new(|| re(r"Tests run: (\d+), Failures: (\d+), Errors: (\d+), Skipped: (\d+)"));
static SELF: LazyLock<Regex> = LazyLock::new(|| re(r"(\d+)/(\d+) (?:gruen|green|ok|passed)"));

pub fn tests(out: &str) -> Option<(u64, u64)> {
    let n = |c: &regex::Captures, i: usize| c.get(i).and_then(|m| m.as_str().parse::<u64>().ok()).unwrap_or(0);
    let cargo: Vec<_> = CARGO.captures_iter(out).collect();
    if !cargo.is_empty() {
        let (p, f) = cargo.iter().fold((0, 0), |(p, f), c| (p + n(c, 1), f + n(c, 2)));
        return Some((p, p + f));
    }
    if let Some(c) = NEXTEST.captures_iter(out).last() {
        return Some((n(&c, 2), n(&c, 1)));
    }
    if let Some(c) = JEST.captures_iter(out).last() {
        return Some((n(&c, 2), n(&c, 3)));
    }
    if let Some(c) = MAVEN.captures_iter(out).last() {
        let (all, bad, skip) = (n(&c, 1), n(&c, 2) + n(&c, 3), n(&c, 4));
        return Some((all - bad - skip, all - skip));
    }
    if let Some(c) = SELF.captures_iter(out).last() {
        return Some((n(&c, 1), n(&c, 2)));
    }
    PYTEST.captures_iter(out).last().map(|c| (n(&c, 2), n(&c, 2) + n(&c, 1)))
}

pub fn result(cmd: &str, ran: &[Ran]) -> Value {
    let mut times: Vec<f64> = ran.iter().map(|r| r.secs).collect();
    times.sort_by(f64::total_cmp);
    let median = times.get(times.len() / 2).copied().unwrap_or(0.0);
    let last = ran.last();
    let code = last.map_or(1, |r| r.code);
    let mut v = json!({"cmd": cmd, "exit": code, "median_s": median, "runs": ran.len(), "times": times, "at": crate::run::now()});
    if let Some((ok, all)) = last.and_then(|r| tests(&r.out)) {
        v["tests"] = json!({"passed": ok, "total": all});
    }
    if code != 0
        && let Some(r) = last
    {
        let lines: Vec<&str> = r.out.trim().lines().collect();
        v["tail"] = json!(lines[lines.len().saturating_sub(TAIL_LINES)..].join("\n"));
    }
    v
}

pub fn line(m: &Value) -> String {
    if m.is_null() {
        return "-".into();
    }
    if let Some(s) = m.get("skipped").and_then(Value::as_str) {
        return format!("skipped: {s}");
    }
    let code = m.get("exit").and_then(Value::as_i64).unwrap_or(1);
    let mut bits = vec![if code == 0 { "OK".to_string() } else { format!("FAILED (exit {code})") }];
    if let Some(s) = m.get("median_s").and_then(Value::as_f64).filter(|s| *s > 0.0) {
        let runs = m.get("runs").and_then(Value::as_i64).unwrap_or(1);
        bits.push(if runs > 1 { format!("{s} s (median of {runs})") } else { format!("{s} s") });
    }
    if let Some(t) = m.get("tests") {
        bits.push(format!("tests {}/{}", t["passed"], t["total"]));
    }
    if let Some(e) = m.get("errors").and_then(Value::as_i64) {
        bits.push(format!("{e} errors, {} warnings", m.get("warnings").and_then(Value::as_i64).unwrap_or(0)));
    }
    bits.join(", ")
}

pub fn compare(a: &Value, b: &Value) -> String {
    let mut out = vec![format!("before: {}", line(a)), format!("after:  {}", line(b))];
    let t = |v: &Value| v.get("median_s").and_then(Value::as_f64).filter(|s| *s > 0.0 && v.get("skipped").is_none());
    if let (Some(x), Some(y)) = (t(a), t(b)) {
        let runs = a.get("runs").and_then(Value::as_i64).unwrap_or(1);
        out.push(format!(
            "time: {:+.1} %{}",
            (y - x) / x * 100.0,
            if runs < 3 { " (noise possible, use --runs 5)" } else { "" }
        ));
    }
    let tests =
        |v: &Value| v.get("tests").map(|t| (t["passed"].as_u64().unwrap_or(0), t["total"].as_u64().unwrap_or(0)));
    if let (Some((pa, ta)), Some((pb, tb))) = (tests(a), tests(b))
        && (pa, ta) != (pb, tb)
    {
        out.push(format!("tests: {pa}/{ta} -> {pb}/{tb}"));
    }
    out.join("\n")
}

pub fn metrics(paths: &[PathBuf]) -> Value {
    let list: Vec<PathBuf> = files::walk(paths).into_iter().filter(|p| files::has_ext(p, files::TEXT_EXT)).collect();
    let (mut loc, mut funcs, mut sum, mut max, mut over) = (0usize, 0usize, 0usize, 0usize, 0usize);
    for f in &list {
        let Some(text) = files::read(f) else { continue };
        loc += text.lines().filter(|l| !l.trim().is_empty()).count();
        if files::has_ext(f, files::CODE_EXT) {
            for fun in functions(&files::ext(f), &text) {
                let ccn = fun.branches + 1;
                funcs += 1;
                sum += ccn;
                max = max.max(ccn);
                over += usize::from(ccn > CCN_WARN);
            }
        }
    }
    let avg = if funcs == 0 { 0.0 } else { (sum as f64 / funcs as f64 * 100.0).round() / 100.0 };
    json!({"files": list.len(), "lines": loc, "functions": funcs, "ccn_avg": avg, "ccn_max": max, "ccn_over": over, "at": crate::run::now()})
}

pub fn metrics_line(m: &Value) -> String {
    format!(
        "{} files, {} lines, {} functions, complexity avg {} max {}, {} over {CCN_WARN}",
        m["files"], m["lines"], m["functions"], m["ccn_avg"], m["ccn_max"], m["ccn_over"]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_counts_are_read_from_every_runner() {
        assert_eq!(
            tests("test result: ok. 5 passed; 0 failed;\ntest result: FAILED. 2 passed; 1 failed;"),
            Some((7, 8))
        );
        assert_eq!(tests("Summary 12 tests run: 11 passed, 1 failed"), Some((11, 12)));
        assert_eq!(tests("Tests:       1 failed, 9 passed, 10 total"), Some((9, 10)));
        assert_eq!(tests("Tests run: 10, Failures: 1, Errors: 0, Skipped: 2"), Some((7, 8)));
        assert_eq!(tests("==== 2 failed, 30 passed in 1.2s ===="), Some((30, 32)));
        assert_eq!(tests("61/61 gruen"), Some((61, 61)));
        assert_eq!(tests("nothing"), None);
        let ok = run("echo hi", Path::new("."), Duration::from_secs(20));
        assert_eq!(ok.code, 0);
        assert!(ok.out.contains("hi"));
        let v = result("x", &[ok]);
        assert_eq!(line(&v).split(',').next(), Some("OK"));
        let a = json!({"exit": 0, "median_s": 2.0, "runs": 5, "tests": {"passed": 3, "total": 4}});
        let b = json!({"exit": 0, "median_s": 1.0, "runs": 5, "tests": {"passed": 4, "total": 4}});
        assert_eq!(
            compare(&a, &b),
            "before: OK, 2 s (median of 5), tests 3/4\nafter:  OK, 1 s (median of 5), tests 4/4\ntime: -50.0 %\ntests: 3/4 -> 4/4"
        );
    }
}

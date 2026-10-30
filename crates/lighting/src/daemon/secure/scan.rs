use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::LazyLock;

use regex::Regex;

const SECRETS: &[(&str, &str)] = &[
    ("GitHub token", r"\bgh[pousr]_[A-Za-z0-9]{20,}"),
    ("API key (sk-/pk-)", r"\b(?:sk|pk|rk)-[A-Za-z0-9_-]{16,}"),
    ("Groq key", r"\bgsk_[A-Za-z0-9]{20,}"),
    ("AWS access key", r"\bAKIA[0-9A-Z]{16}\b"),
    ("Slack token", r"\bxox[abpr]-[A-Za-z0-9-]{10,}"),
    ("Google API key", r"\bAIza[0-9A-Za-z_-]{35}\b"),
    ("Discord bot token", r"\b[MN][A-Za-z\d]{23,25}\.[\w-]{6}\.[\w-]{27,}"),
    ("private key", r"-----BEGIN (?:RSA |EC |OPENSSH |DSA |ENCRYPTED )?PRIVATE KEY-----"),
    ("JWT", r"\beyJ[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}"),
];
const RISKY: &[(&str, &str, &str)] = &[
    (
        r#"(?i)"\s*(?:select|insert\s+into|update|delete\s+from|replace\s+into)\b[^"]*"\s*\+"#,
        "SQL built from joined text",
        "use a PreparedStatement with ? placeholders",
    ),
    (
        r#"(?i)\+\s*"\s*(?:where|and|or|values|set|order by|limit)\b"#,
        "SQL built from joined text",
        "use a PreparedStatement with ? placeholders",
    ),
    (r#"(?i)\.execute\w*\(\s*f["']"#, "SQL from an f-string", "pass the values as parameters: execute(sql, (a, b))"),
    (r"(?i)\.query\(\s*`[^`]*\$\{", "SQL from a template string", "use placeholders and pass the values separately"),
    (
        r"Runtime\.getRuntime\(\)\.exec\([^)]*\+",
        "shell command built from joined text",
        "use ProcessBuilder with a fixed program and separate arguments",
    ),
    (r"subprocess\.\w+\([^)]*shell\s*=\s*True", "shell=True runs text through the shell", "pass a list of arguments"),
    (r"(?:^|[^\w.])eval\(", "eval runs text as code", "parse the data instead"),
];
const CONFIG_EXT: &[&str] = &["yml", "yaml", "json", "properties", "env", "toml", "ini", "conf", "cfg", "xml"];
const PLACEHOLDER: &[&str] = &[
    "changeme",
    "change_me",
    "password",
    "secret",
    "token",
    "example",
    "your",
    "xxx",
    "***",
    "<",
    "${",
    "{{",
    "%s",
    "null",
    "none",
    "todo",
];
const MAX_BYTES: u64 = 1_000_000;
const HISTORY_LINES: usize = 2_000_000;

static FOUND: LazyLock<Vec<(&'static str, Regex)>> =
    LazyLock::new(|| SECRETS.iter().map(|(name, p)| (*name, Regex::new(p).expect("secret"))).collect());
static ASSIGN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b(password|passwd|pwd|secret|token|api[_-]?key|license[_-]?key)["']?\s*[:=]\s*["']?([^\s"',;]{6,})"#,
    )
    .expect("assign")
});
static RISKS: LazyLock<Vec<(Regex, &'static str, &'static str)>> =
    LazyLock::new(|| RISKY.iter().map(|(p, why, fix)| (Regex::new(p).expect("risky"), *why, *fix)).collect());

pub struct Finding {
    pub file: String,
    pub line: usize,
    pub why: String,
    pub fix: String,
}

fn masked(value: &str) -> String {
    let head: String = value.chars().take(4).collect();
    format!("{head}…")
}

fn placeholder(value: &str) -> bool {
    let low = value.to_lowercase();
    PLACEHOLDER.iter().any(|p| low.starts_with(p))
        || value.contains(['(', ')', '$'])
        || value.chars().all(|c| c == '*' || c == 'x')
}

pub fn secrets_in_line(line: &str, config: bool) -> Vec<String> {
    let mut out: Vec<String> = FOUND
        .iter()
        .filter_map(|(name, rx)| rx.find(line).map(|m| format!("{name} ({})", masked(m.as_str()))))
        .collect();
    if out.is_empty()
        && config
        && let Some(m) = ASSIGN.captures(line)
        && !placeholder(&m[2])
    {
        out.push(format!("{} in plain text ({})", m[1].to_lowercase(), masked(&m[2])));
    }
    out
}

pub fn risky_in_line(line: &str) -> Option<(&'static str, &'static str)> {
    let code = line.trim_start();
    if code.starts_with("//") || code.starts_with('#') || code.starts_with('*') {
        return None;
    }
    RISKS.iter().find(|(rx, _, _)| rx.is_match(line)).map(|(_, why, fix)| (*why, *fix))
}

fn git(root: &Path, args: &[&str]) -> Command {
    let mut cmd = Command::new("git");
    cmd.args(["-c", "core.quotePath=false", "-C"]).arg(root).args(args).stdin(Stdio::null()).stderr(Stdio::null());
    crate::v2::hide(&mut cmd);
    cmd
}

pub fn files(root: &Path) -> Vec<String> {
    let out = git(root, &["ls-files", "-z", "-co", "--exclude-standard"]).output().ok().filter(|o| o.status.success());
    match out {
        Some(o) => String::from_utf8_lossy(&o.stdout).split('\0').filter(|n| !n.is_empty()).map(String::from).collect(),
        None => {
            let mut found = Vec::new();
            walk(root, root, &mut found);
            found
        }
    }
}

fn walk(root: &Path, dir: &Path, found: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if !name.starts_with('.')
                && !["node_modules", "target", "build", "out", "dist", "venv"].contains(&name.as_str())
            {
                walk(root, &path, found);
            }
        } else if let Ok(rel) = path.strip_prefix(root) {
            found.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
}

fn config_file(name: &str) -> bool {
    let low = name.to_lowercase();
    low.rsplit('/').next().is_some_and(|f| f.starts_with(".env"))
        || low.rsplit_once('.').is_some_and(|(_, ext)| CONFIG_EXT.contains(&ext))
}

pub fn scan_files(root: &Path, names: &[String]) -> (Vec<Finding>, Vec<Finding>) {
    let (mut secrets, mut risky) = (Vec::new(), Vec::new());
    for name in names {
        let low = name.to_lowercase();
        if low.ends_with(".lock") || low.ends_with("package-lock.json") || low.contains("/test") {
            continue;
        }
        let path = root.join(name);
        if !std::fs::metadata(&path).is_ok_and(|m| m.len() <= MAX_BYTES) {
            continue;
        }
        let Ok(raw) = std::fs::read(&path) else { continue };
        if raw.iter().take(8192).any(|b| *b == 0) {
            continue;
        }
        let text = String::from_utf8_lossy(&raw);
        let config = config_file(name);
        if low.rsplit('/').next().is_some_and(|f| f == ".env" || (f.starts_with(".env.") && !f.ends_with("example"))) {
            secrets.push(Finding {
                file: name.clone(),
                line: 1,
                why: "an .env file is in the repo".into(),
                fix: "add it to .gitignore, keep an .env.example without values".into(),
            });
        }
        for (i, line) in text.lines().enumerate() {
            for why in secrets_in_line(line, config) {
                secrets.push(Finding {
                    file: name.clone(),
                    line: i + 1,
                    why,
                    fix: "move it to an environment variable or an ignored file, then rotate the key".into(),
                });
            }
            if !config && let Some((why, fix)) = risky_in_line(line) {
                risky.push(Finding { file: name.clone(), line: i + 1, why: why.into(), fix: fix.into() });
            }
        }
        if (low.ends_with(".java") || low.ends_with(".kt"))
            && text.contains("onCommand(")
            && !text.contains("hasPermission(")
            && let Some(at) = text.lines().position(|l| l.contains("onCommand("))
        {
            risky.push(Finding {
                file: name.clone(),
                line: at + 1,
                why: "command without a permission check in this file".into(),
                fix: "check sender.hasPermission(...) or set permission: in plugin.yml".into(),
            });
        }
    }
    (secrets, risky)
}

pub fn scan_history(root: &Path) -> Vec<Finding> {
    let mut cmd = git(root, &["log", "-p", "--all", "--no-color", "-U0", "--format=commit %h"]);
    cmd.stdout(Stdio::piped());
    let Ok(mut child) = cmd.spawn() else { return Vec::new() };
    let Some(out) = child.stdout.take() else { return Vec::new() };
    let (mut found, mut commit, mut file) = (Vec::new(), String::new(), String::new());
    for line in BufReader::new(out).lines().map_while(Result::ok).take(HISTORY_LINES) {
        if let Some(h) = line.strip_prefix("commit ") {
            commit = h.trim().to_string();
        } else if let Some(f) = line.strip_prefix("+++ ") {
            file = f.strip_prefix("b/").unwrap_or(f).to_string();
        } else if let Some(added) = line.strip_prefix('+') {
            for why in secrets_in_line(added, config_file(&file)) {
                found.push(Finding {
                    file: format!("{file} (commit {commit})"),
                    line: 0,
                    why,
                    fix: "rotate the key; removing it needs a history rewrite (git filter-repo)".into(),
                });
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_and_risky_lines_are_found_without_placeholders() {
        let fake = format!("token = \"ghp_{}\"", "a1B2".repeat(6));
        assert_eq!(secrets_in_line(&fake, false), vec!["GitHub token (ghp_…)"]);
        assert_eq!(secrets_in_line("password: hunter2secret", true), vec!["password in plain text (hunt…)"]);
        assert!(secrets_in_line("password: changeme", true).is_empty());
        assert!(secrets_in_line("password: ${DB_PASS}", true).is_empty());
        assert!(secrets_in_line("password: hunter2secret", false).is_empty());
        let sql = r#"stmt.executeQuery("SELECT * FROM users WHERE name = '" + name + "'");"#;
        assert_eq!(risky_in_line(sql).map(|r| r.0), Some("SQL built from joined text"));
        assert!(risky_in_line(r#"ps = conn.prepareStatement("SELECT * FROM users WHERE name = ?");"#).is_none());
        assert!(risky_in_line("// \"select x\" + y").is_none());
        assert_eq!(risky_in_line("cur.execute(f\"select {x}\")").map(|r| r.0), Some("SQL from an f-string"));
        assert!(config_file("plugins/x/config.yml") && config_file(".env.local") && !config_file("Main.java"));
    }
}

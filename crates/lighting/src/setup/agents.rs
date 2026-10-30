use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::place::copy_skill;

pub const NAMES: &[&str] = &[
    "antigravity",
    "bionic",
    "claude-desktop",
    "cline",
    "codex",
    "cursor",
    "gemini",
    "lmstudio",
    "roo",
    "vscode",
    "windsurf",
    "zed",
];

fn user_home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default()
}

fn app_data(home: &Path) -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData").join("Roaming"))
    } else if cfg!(target_os = "macos") {
        home.join("Library").join("Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".config"))
    }
}

pub fn config_file(name: &str) -> Option<PathBuf> {
    let home = user_home();
    let app = app_data(&home);
    let vsext = app.join("Code").join("User").join("globalStorage");
    let zed = if cfg!(windows) {
        app.join("Zed")
    } else if cfg!(target_os = "macos") {
        home.join(".config").join("zed")
    } else {
        app.join("zed")
    };
    Some(match name {
        "antigravity" => home.join(".gemini").join("config").join("mcp_config.json"),
        "claude-desktop" => app.join("Claude").join("claude_desktop_config.json"),
        "cline" => vsext.join("saoudrizwan.claude-dev").join("settings").join("cline_mcp_settings.json"),
        "codex" => home.join(".codex").join("config.toml"),
        "cursor" => home.join(".cursor").join("mcp.json"),
        "gemini" => home.join(".gemini").join("settings.json"),
        "lmstudio" | "bionic" => home.join(".lmstudio").join("mcp.json"),
        "roo" => vsext.join("rooveterinaryinc.roo-cline").join("settings").join("mcp_settings.json"),
        "vscode" => app.join("Code").join("User").join("mcp.json"),
        "windsurf" => home.join(".codeium").join("windsurf").join("mcp_config.json"),
        "zed" => zed.join("settings.json"),
        _ => return None,
    })
}

fn entry(exe: &Path) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("command".into(), json!(exe.display().to_string()));
    m.insert("args".into(), json!(["mcp"]));
    m
}

fn eol_of(path: &Path) -> &'static str {
    if fs::read(path).is_ok_and(|b| b.windows(2).any(|w| w == b"\r\n")) { "\r\n" } else { "\n" }
}

pub fn merge_json(path: &Path, key: &str, value: Map<String, Value>) -> io::Result<()> {
    let mut data = match fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => json!({}),
        Ok(text) => serde_json::from_str::<Value>(&text).ok().filter(Value::is_object).ok_or_else(|| {
            io::Error::other(format!("{} is not valid JSON, fix it or add the server by hand", path.display()))
        })?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e),
    };
    if !data[key].is_object() {
        data[key] = json!({});
    }
    data[key]["lighting"] = Value::Object(value);
    let eol = eol_of(path);
    let text = serde_json::to_string_pretty(&data).map_err(io::Error::other)?.replace('\n', eol);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, text)
}

pub fn merge_toml(path: &Path, exe: &Path) -> io::Result<()> {
    let exe = exe.display().to_string().replace('\\', "\\\\");
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let eol = eol_of(path);
    let mut kept = Vec::new();
    let mut skipping = false;
    for line in text.lines() {
        if line.trim_end() == "[mcp_servers.lighting]" {
            skipping = true;
            continue;
        }
        if skipping && line.starts_with('[') {
            skipping = false;
        }
        if !skipping {
            kept.push(line);
        }
    }
    let rest = kept.join("\n").trim_end().to_string();
    let block = format!("[mcp_servers.lighting]\ncommand = \"{exe}\"\nargs = [\"mcp\"]\n");
    let out = if rest.is_empty() { block } else { format!("{rest}\n\n{block}") };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, out.replace('\n', eol))
}

fn gemini_skills(home: &Path) -> [PathBuf; 4] {
    let g = home.join(".gemini");
    [
        g.join("skills").join("lighting"),
        g.join("antigravity").join("global_skills").join("lighting"),
        g.join("antigravity").join("skills").join("lighting"),
        g.join("antigravity-cli").join("skills").join("lighting"),
    ]
}

pub fn refresh_skills(root: Option<&Path>) {
    let home = user_home();
    let bionic = home.join(".lmstudio").join("skills").join("lighting");
    for dst in std::iter::once(bionic).chain(gemini_skills(&home)) {
        if dst.exists() {
            let _ = copy_skill(root, &dst);
        }
    }
}

pub fn install(name: &str, exe: &Path, root: Option<&Path>) -> io::Result<()> {
    let home = user_home();
    let path =
        config_file(name).ok_or_else(|| io::Error::other(format!("usage: lighting install <{}>", NAMES.join("|"))))?;
    let base = entry(exe);
    match name {
        "codex" => merge_toml(&path, exe),
        "cline" => {
            let mut e = base;
            e.insert("disabled".into(), json!(false));
            merge_json(&path, "mcpServers", e)
        }
        "vscode" => {
            let mut e = base;
            e.insert("type".into(), json!("stdio"));
            merge_json(&path, "servers", e)
        }
        "zed" => {
            let mut e = base;
            e.insert("source".into(), json!("custom"));
            merge_json(&path, "context_servers", e)
        }
        "lmstudio" | "bionic" => {
            merge_json(&path, "mcpServers", base)?;
            copy_skill(root, &home.join(".lmstudio").join("skills").join("lighting"))
        }
        "antigravity" => {
            merge_json(&path, "mcpServers", base.clone())?;
            let old = home.join(".gemini").join("antigravity").join("mcp_config.json");
            if old.exists() {
                merge_json(&old, "mcpServers", base)?;
            }
            let [first, rest @ ..] = gemini_skills(&home);
            copy_skill(root, &first)?;
            for p in rest.iter().filter(|p| p.parent().and_then(Path::parent).is_some_and(Path::exists)) {
                copy_skill(root, p)?;
            }
            Ok(())
        }
        _ => merge_json(&path, "mcpServers", base),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_entries_merge_into_existing_files_and_keep_line_endings() {
        let dir = std::env::temp_dir().join(format!("lt-agents-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let exe = Path::new(r"C:\x\lighting.exe");
        let json_file = dir.join("mcp.json");
        fs::write(&json_file, "{\r\n  \"theme\": \"dark\",\r\n  \"mcpServers\": {\"other\": {}}\r\n}").unwrap();
        merge_json(&json_file, "mcpServers", entry(exe)).unwrap();
        let text = fs::read_to_string(&json_file).unwrap();
        assert!(text.contains("\r\n") && !text.replace("\r\n", "").contains('\n'));
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["mcpServers"]["other"], json!({}));
        assert_eq!(v["mcpServers"]["lighting"]["args"], json!(["mcp"]));
        fs::write(&json_file, "{oops").unwrap();
        assert!(merge_json(&json_file, "mcpServers", entry(exe)).unwrap_err().to_string().contains("not valid JSON"));
        let toml = dir.join("config.toml");
        fs::write(&toml, "model = \"x\"\n\n[mcp_servers.lighting]\ncommand = \"old\"\nargs = []\n\n[other]\na = 1\n")
            .unwrap();
        merge_toml(&toml, exe).unwrap();
        merge_toml(&toml, exe).unwrap();
        let text = fs::read_to_string(&toml).unwrap();
        assert_eq!(text.matches("[mcp_servers.lighting]").count(), 1);
        assert!(
            text.contains("[other]\na = 1")
                && !text.contains("\"old\"")
                && text.contains(r#"command = "C:\\x\\lighting.exe""#)
        );
        assert!(config_file("nope").is_none() && NAMES.iter().all(|n| config_file(n).is_some()));
        let _ = fs::remove_dir_all(&dir);
    }
}

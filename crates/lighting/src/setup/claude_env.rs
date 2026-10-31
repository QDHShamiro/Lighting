use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

const BIN: &str = "LIGHTING_BIN";
const CLIENT: &str = "LIGHTING_CLIENT";

pub fn settings_file() -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home.unwrap_or_default().join(".claude"))
        .join("settings.json")
}

fn slashed(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn wanted(exe: &Path) -> Vec<(&'static str, String)> {
    let mut out = vec![(BIN, exe.parent().map(slashed).unwrap_or_default())];
    if !cfg!(windows) {
        out.push((CLIENT, exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    }
    out
}

fn load(file: &Path) -> Result<Map<String, Value>, String> {
    let raw = match fs::read_to_string(file) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(format!("cannot read {}: {e}", file.display())),
    };
    match serde_json::from_str(&raw) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(format!("{} is not a JSON object, left alone", file.display())),
    }
}

fn save(file: &Path, map: Map<String, Value>) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&Value::Object(map)).map_err(|e| e.to_string())? + "\n";
    let tmp = file.with_extension("json.lighting-tmp");
    fs::write(&tmp, text)
        .and_then(|()| fs::rename(&tmp, file))
        .map_err(|e| format!("cannot write {}: {e}", file.display()))
}

pub fn point(file: &Path, exe: &Path) -> Result<bool, String> {
    if !file.parent().is_some_and(Path::is_dir) {
        return Ok(false);
    }
    let mut map = load(file)?;
    let env = map.entry("env").or_insert_with(|| Value::Object(Map::new()));
    let Value::Object(env) = env else { return Err(format!("env in {} is not an object, left alone", file.display())) };
    let mut changed = false;
    for (key, value) in wanted(exe) {
        if env.get(key).and_then(Value::as_str) != Some(value.as_str()) {
            env.insert(key.into(), Value::String(value));
            changed = true;
        }
    }
    if changed {
        save(file, map)?;
    }
    Ok(changed)
}

pub fn unpoint(file: &Path) -> Result<bool, String> {
    if !file.is_file() {
        return Ok(false);
    }
    let mut map = load(file)?;
    let Some(Value::Object(env)) = map.get_mut("env") else { return Ok(false) };
    let gone = [BIN, CLIENT].iter().filter(|k| env.shift_remove(**k).is_some()).count();
    if gone > 0 {
        save(file, map)?;
    }
    Ok(gone > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_claude_env_points_at_the_bin_folder_and_keeps_the_rest() {
        let dir = std::env::temp_dir().join(format!("lt-claude-env-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("settings.json");
        fs::write(&file, "{\n  \"model\": \"opus\",\n  \"env\": {\n    \"A\": \"ü\"\n  },\n  \"z\": [1, 2]\n}\n")
            .unwrap();
        let exe = dir.join("bin").join(if cfg!(windows) { "lighting.exe" } else { "lighting" });
        assert!(point(&file, &exe).unwrap());
        assert!(!point(&file, &exe).unwrap());
        let map = load(&file).unwrap();
        let keys: Vec<&String> = map.keys().collect();
        assert_eq!(keys, ["model", "env", "z"]);
        let env = map["env"].as_object().unwrap();
        assert_eq!(env["A"], "ü");
        assert_eq!(env[BIN].as_str(), Some(slashed(&dir.join("bin")).as_str()));
        assert_eq!(env.contains_key(CLIENT), !cfg!(windows));
        assert!(unpoint(&file).unwrap());
        assert!(!unpoint(&file).unwrap());
        assert_eq!(load(&file).unwrap()["env"].as_object().unwrap().len(), 1);
        assert!(!point(&dir.join("missing").join("settings.json"), &exe).unwrap());
        fs::write(&file, "[1]").unwrap();
        assert!(point(&file, &exe).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}

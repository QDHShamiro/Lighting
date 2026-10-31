use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const SKIP_DIRS: &[&str] = &[
    ".git",
    "target",
    "build",
    "out",
    "dist",
    "node_modules",
    ".gradle",
    ".idea",
    ".vscode",
    "__pycache__",
    ".venv",
    "venv",
    ".mvn",
    ".pytest_cache",
    ".next",
    "coverage",
    ".claude",
];
pub const CODE_EXT: &[&str] =
    &["rs", "java", "kt", "py", "js", "ts", "tsx", "jsx", "mjs", "cjs", "go", "cs", "lua", "vue", "svelte", "sh"];
pub const WEB_EXT: &[&str] = &["html", "htm", "css", "scss", "vue", "svelte", "jsx", "tsx"];
pub const TEXT_EXT: &[&str] = &[
    "rs",
    "java",
    "kt",
    "py",
    "js",
    "ts",
    "tsx",
    "jsx",
    "mjs",
    "cjs",
    "go",
    "cs",
    "lua",
    "vue",
    "svelte",
    "sh",
    "html",
    "htm",
    "css",
    "scss",
    "md",
    "yml",
    "yaml",
    "json",
    "toml",
    "xml",
    "properties",
    "sql",
    "ps1",
    "bat",
    "cmd",
    "txt",
    "gradle",
    "kts",
    "svg",
];
pub const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "svg"];
const MARKERS: &[&str] = &[
    "Cargo.toml",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "package.json",
    "pyproject.toml",
    "requirements.txt",
    "setup.py",
    "SKILL.md",
    "fabric.mod.json",
    ".git",
];
const WALK_MAX: usize = 20_000;
const READ_MAX: u64 = 2_000_000;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum Pack {
    Code,
    Rust,
    JavaPlugin,
    Mod,
    Web,
    Python,
    Docs,
    Images,
    McGui,
    Skill,
}

impl Pack {
    pub const ALL: [Pack; 10] = [
        Pack::Code,
        Pack::Rust,
        Pack::JavaPlugin,
        Pack::Mod,
        Pack::Web,
        Pack::Python,
        Pack::Docs,
        Pack::Images,
        Pack::McGui,
        Pack::Skill,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Pack::Code => "code",
            Pack::Rust => "rust",
            Pack::JavaPlugin => "java-plugin",
            Pack::Mod => "mod",
            Pack::Web => "web",
            Pack::Python => "python",
            Pack::Docs => "docs",
            Pack::Images => "images",
            Pack::McGui => "mc-gui",
            Pack::Skill => "skill",
        }
    }

    pub fn from_name(name: &str) -> Option<Pack> {
        Pack::ALL.into_iter().find(|p| p.name() == name.trim().to_lowercase())
    }
}

pub fn ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

pub fn has_ext(p: &Path, list: &[&str]) -> bool {
    list.contains(&ext(p).as_str())
}

pub fn walk(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut todo: Vec<PathBuf> = paths.to_vec();
    while let Some(p) = todo.pop() {
        if out.len() >= WALK_MAX {
            break;
        }
        if p.is_file() {
            out.push(p);
            continue;
        }
        let Ok(list) = fs::read_dir(&p) else { continue };
        for e in list.flatten() {
            let path = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            match e.file_type() {
                Ok(t) if t.is_dir() => {
                    if !SKIP_DIRS.contains(&name.as_str()) {
                        todo.push(path);
                    }
                }
                Ok(t) if t.is_file() => out.push(path),
                _ => {}
            }
        }
    }
    out.sort();
    out
}

pub fn read(p: &Path) -> Option<String> {
    if fs::metadata(p).ok()?.len() > READ_MAX {
        return None;
    }
    let raw = fs::read(p).ok()?;
    if raw.iter().take(8192).any(|b| *b == 0) {
        return None;
    }
    Some(String::from_utf8(raw).unwrap_or_else(|e| e.into_bytes().iter().map(|b| *b as char).collect()))
}

pub fn root_of(p: &Path) -> PathBuf {
    let start = if p.is_dir() { p.to_path_buf() } else { p.parent().map(Path::to_path_buf).unwrap_or_default() };
    for d in start.ancestors() {
        if MARKERS.iter().any(|m| d.join(m).exists()) {
            return d.to_path_buf();
        }
    }
    start
}

pub fn is_test(p: &Path) -> bool {
    let s = slashed(p).to_lowercase();
    s.contains("/tests/")
        || s.contains("/test/")
        || s.contains(".test.")
        || s.contains("_test.")
        || s.contains(".spec.")
}

pub fn slashed(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

pub fn rel(root: &Path, p: &Path) -> String {
    slashed(p.strip_prefix(root).unwrap_or(p))
}

pub fn packs_of(root: &Path, files: &[PathBuf]) -> BTreeSet<Pack> {
    let mut out = BTreeSet::new();
    let at = |m: &str| root.join(m).exists();
    if at("Cargo.toml") {
        out.insert(Pack::Rust);
    }
    if at("package.json") || at("index.html") {
        out.insert(Pack::Web);
    }
    if at("fabric.mod.json") || at("src/main/resources/fabric.mod.json") {
        out.insert(Pack::Mod);
    }
    if at("SKILL.md") {
        out.insert(Pack::Skill);
    }
    for f in files {
        let e = ext(f);
        let name = f.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        let path = slashed(f).to_lowercase();
        match e.as_str() {
            "rs" => {
                out.insert(Pack::Rust);
            }
            "py" => {
                out.insert(Pack::Python);
            }
            "md" => {
                out.insert(Pack::Docs);
            }
            _ => {}
        }
        if CODE_EXT.contains(&e.as_str()) {
            out.insert(Pack::Code);
        }
        if WEB_EXT.contains(&e.as_str()) {
            out.insert(Pack::Web);
        }
        if IMAGE_EXT.contains(&e.as_str()) {
            out.insert(Pack::Images);
        }
        if name == "plugin.yml" || name == "paper-plugin.yml" {
            out.insert(Pack::JavaPlugin);
        }
        if name == "fabric.mod.json" {
            out.insert(Pack::Mod);
        }
        if (e == "yml" || e == "yaml") && (path.contains("/gui/") || path.contains("/guis/") || name == "messages.yml")
        {
            out.insert(Pack::McGui);
        }
    }
    out
}

pub fn quick_packs(root: &Path) -> BTreeSet<Pack> {
    let mut out = BTreeSet::new();
    let at = |m: &str| root.join(m).exists();
    if at("Cargo.toml") {
        out.insert(Pack::Rust);
    }
    if at("package.json") || at("index.html") {
        out.insert(Pack::Web);
    }
    if at("src/main/resources/plugin.yml") || at("src/main/resources/paper-plugin.yml") || at("plugin.yml") {
        out.insert(Pack::JavaPlugin);
        out.insert(Pack::McGui);
    }
    if at("fabric.mod.json") || at("src/main/resources/fabric.mod.json") {
        out.insert(Pack::Mod);
    }
    if at("pyproject.toml") || at("requirements.txt") || at("setup.py") {
        out.insert(Pack::Python);
    }
    if at("SKILL.md") {
        out.insert(Pack::Skill);
    }
    if !out.is_empty() {
        out.insert(Pack::Code);
        out.insert(Pack::Docs);
        out.insert(Pack::Images);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_follow_the_files() {
        let root = Path::new("/nowhere");
        let files: Vec<PathBuf> =
            ["a/src/main.rs", "a/src/main/resources/plugin.yml", "a/gui/shop.yml", "a/x.png", "a/README.md"]
                .iter()
                .map(PathBuf::from)
                .collect();
        let got: Vec<&str> = packs_of(root, &files).into_iter().map(Pack::name).collect();
        assert_eq!(got, ["code", "rust", "java-plugin", "docs", "images", "mc-gui"]);
        assert_eq!(Pack::from_name(" Web "), Some(Pack::Web));
        assert!(is_test(Path::new("crates/x/tests/a.rs")) && !is_test(Path::new("src/a.rs")));
        assert_eq!(rel(Path::new("/p"), Path::new("/p/src/a.rs")), "src/a.rs");
    }
}

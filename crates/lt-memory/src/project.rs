use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

const BUILD_FILES: &[&str] = &[
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "requirements.txt",
    "go.mod",
    "setup.py",
    "composer.json",
    "Gemfile",
    "CMakeLists.txt",
    "Makefile",
];
const MONOREPO_MIN: usize = 3;
const CACHE_MAX: usize = 8192;
const SCRATCH: &[&str] = &["AppData", ".claude", ".cache", ".local", "Downloads", ".lighting"];

static NAMES: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static MONO: LazyLock<Mutex<HashMap<PathBuf, bool>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

fn key(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    let s = s.trim_end_matches('/');
    if cfg!(windows) { s.to_lowercase() } else { s.to_string() }
}

fn same(a: &Path, b: &Path) -> bool {
    key(a) == key(b)
}

fn under(p: &Path, root: &Path) -> bool {
    let (p, root) = (key(p), key(root));
    p == root || p.starts_with(&format!("{root}/"))
}

fn home() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env::var_os(var).map(PathBuf::from).unwrap_or_default()
}

fn built(d: &Path) -> bool {
    BUILD_FILES.iter().any(|f| d.join(f).exists())
}

fn monorepo(root: &Path) -> bool {
    if let Some(known) = MONO.lock().ok().and_then(|m| m.get(root).copied()) {
        return known;
    }
    let found = root
        .read_dir()
        .map(|dir| dir.flatten().filter(|e| e.path().is_dir() && built(&e.path())).count() >= MONOREPO_MIN)
        .unwrap_or(false);
    if let Ok(mut m) = MONO.lock() {
        m.insert(root.to_path_buf(), found);
    }
    found
}

fn name(p: &Path) -> String {
    p.file_name().map_or_else(|| p.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned())
}

fn scratch(p: &Path, home: &Path) -> bool {
    let tmp = env::temp_dir();
    same(p, home) || under(p, &tmp) || SCRATCH.iter().any(|s| under(p, &home.join(s)))
}

pub fn root_of(p: &Path) -> Option<PathBuf> {
    let home = home();
    let git = p.ancestors().take_while(|d| !same(d, &home)).find(|d| d.join(".git").exists());
    let Some(root) = git else {
        return p.ancestors().find(|d| !same(d, &home) && built(d)).map(Path::to_path_buf);
    };
    if !same(p, root)
        && monorepo(root)
        && let Some(first) = p.strip_prefix(root).ok().and_then(|rest| rest.components().next())
    {
        let child = root.join(first);
        if built(&child) {
            return Some(child);
        }
    }
    Some(root.to_path_buf())
}

fn find(p: &Path) -> String {
    match root_of(p) {
        Some(root) => name(&root),
        None if scratch(p, &home()) => String::new(),
        None => name(p),
    }
}

pub fn project_of(cwd: &str) -> String {
    if cwd.is_empty() {
        return String::new();
    }
    if let Some(known) = NAMES.lock().ok().and_then(|m| m.get(cwd).cloned()) {
        return known;
    }
    let found = find(Path::new(cwd));
    if let Ok(mut m) = NAMES.lock() {
        if m.len() >= CACHE_MAX {
            m.clear();
        }
        m.insert(cwd.to_string(), found.clone());
    }
    found
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn projects_come_from_git_roots_build_files_and_monorepos() {
        let base = home().join(format!("ltm-proj-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let repo = base.join("Shop");
        for app in ["api", "web", "admin"] {
            fs::create_dir_all(repo.join(app).join("src")).unwrap();
            fs::write(repo.join(app).join("package.json"), "{}").unwrap();
        }
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(repo.join("docs")).unwrap();
        let plain = base.join("Tool").join("deep");
        fs::create_dir_all(&plain).unwrap();
        fs::write(base.join("Tool").join("Cargo.toml"), "").unwrap();
        let s = |p: &Path| p.to_string_lossy().into_owned();
        assert_eq!(project_of(&s(&repo.join("web").join("src"))), "web");
        assert_eq!(project_of(&s(&repo.join("docs"))), "Shop");
        assert_eq!(project_of(&s(&repo)), "Shop");
        assert_eq!(project_of(&s(&plain)), "Tool");
        assert_eq!(project_of(&s(&home())), "");
        assert_eq!(project_of(&s(&home().join("Downloads").join("x"))), "");
        assert_eq!(project_of(""), "");
        let _ = fs::remove_dir_all(&base);
    }
}

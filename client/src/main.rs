use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::windows::io::{AsRawHandle, RawHandle};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const DETACHED: u32 = 0x0000_0200 | 0x0800_0000;
const BREAKAWAY: u32 = 0x0100_0000;
const ERROR_PIPE_BUSY: i32 = 231;
const LOCAL: &[&str] = &[
    "", "help", "-h", "--help", "version", "--version", "daemon", "host", "mcp", "setup", "uninstall", "install",
];

fn home() -> PathBuf {
    match env::var_os("LIGHTING_HOME") {
        Some(h) if !h.is_empty() => PathBuf::from(h),
        _ => PathBuf::from(env::var_os("USERPROFILE").unwrap_or_default()).join(".lighting"),
    }
}

fn root(home: &Path) -> PathBuf {
    let exe = env::current_exe().unwrap_or_default();
    let text = exe.display().to_string();
    let exe = PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text));
    let own = exe
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    if own.join("lighting").join("__init__.py").exists() {
        return own;
    }
    fs::read_to_string(home.join("client-root"))
        .map(|s| PathBuf::from(s.trim()))
        .ok()
        .filter(|p| p.join("lighting").join("__init__.py").exists())
        .unwrap_or(own)
}

fn norm(s: &str) -> String {
    s.trim().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

fn scripts(home: &Path, exe: &str) -> PathBuf {
    home.join("venv").join("Scripts").join(exe)
}

fn mtime(p: &Path) -> Option<std::time::SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn ready(home: &Path, root: &Path) -> bool {
    let stamp = mtime(&home.join("venv").join(".lighting-stamp"));
    let fresh = |p: PathBuf| match (stamp, mtime(&p)) {
        (_, None) => true,
        (Some(s), Some(f)) => s >= f,
        (None, Some(_)) => false,
    };
    scripts(home, "pythonw.exe").exists()
        && fresh(root.join("requirements.txt"))
        && fresh(root.join(".claude-plugin").join("plugin.json"))
        && fs::read_to_string(home.join("client-root"))
            .map(|s| norm(&s) == norm(&root.display().to_string()))
            .unwrap_or(false)
}

fn python(home: &Path, root: &Path, args: &[String]) -> ! {
    let py = scripts(home, "python.exe");
    let mut cmd = if py.exists() {
        let mut c = Command::new(&py);
        c.args(["-X", "utf8", "-m", "lighting"]);
        c
    } else {
        let mut c = Command::new("uv");
        c.args(["run", "--quiet", "--no-project", "--python", ">=3.11", "python", "-X", "utf8", "-m", "lighting"]);
        c
    };
    cmd.args(args)
        .env("PYTHONPATH", root)
        .env("LIGHTING_ROOT", root)
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8");
    match cmd.status() {
        Ok(s) => process::exit(s.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("lighting: cannot start python ({e}); install uv: https://docs.astral.sh/uv/");
            process::exit(127)
        }
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn SetHandleInformation(handle: RawHandle, mask: u32, flags: u32) -> i32;
}

fn private_std_handles() {
    for h in [io::stdin().as_raw_handle(), io::stdout().as_raw_handle(), io::stderr().as_raw_handle()] {
        if !h.is_null() {
            unsafe { SetHandleInformation(h, 1, 0) };
        }
    }
}

fn start_daemon(home: &Path, root: &Path) {
    private_std_handles();
    let _ = fs::create_dir_all(home);
    let log = OpenOptions::new().create(true).append(true).open(home.join("daemon.log"));
    let spawn = |flags: u32| {
        let mut c = Command::new(scripts(home, "pythonw.exe"));
        c.args(["-X", "utf8", "-m", "lighting", "daemon"])
            .env("PYTHONPATH", root)
            .env("LIGHTING_ROOT", root)
            .current_dir(home)
            .stdin(Stdio::null())
            .creation_flags(flags);
        match log.as_ref().ok().and_then(|l| Some((l.try_clone().ok()?, l.try_clone().ok()?))) {
            Some((a, b)) => c.stdout(a).stderr(b),
            None => c.stdout(Stdio::null()).stderr(Stdio::null()),
        };
        c.spawn()
    };
    if spawn(DETACHED | BREAKAWAY).is_err() {
        let _ = spawn(DETACHED);
    }
}

fn pipe_name() -> String {
    let user = env::var("USERNAME").unwrap_or_else(|_| "user".into()).to_lowercase();
    format!(r"\\.\pipe\lighting-{user}")
}

fn open_pipe() -> io::Result<File> {
    OpenOptions::new().read(true).write(true).open(pipe_name())
}

fn exchange(home: &Path, root: &Path, req: &[u8]) -> Vec<u8> {
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut spawned = false;
    loop {
        match open_pipe() {
            Ok(mut f) => {
                let mut buf = Vec::new();
                if f.write_all(req).is_ok() && f.read_to_end(&mut buf).is_ok() && !buf.is_empty() {
                    return buf;
                }
                eprintln!("lighting: daemon closed the connection, see {}", home.join("daemon.log").display());
                process::exit(1);
            }
            Err(e) => {
                let busy = e.raw_os_error() == Some(ERROR_PIPE_BUSY);
                if !busy && !spawned {
                    start_daemon(home, root);
                    spawned = true;
                }
                if Instant::now() > deadline {
                    eprintln!("lighting: daemon did not start, see {}", home.join("daemon.log").display());
                    process::exit(1);
                }
                sleep(Duration::from_millis(if busy { 15 } else { 40 }));
            }
        }
    }
}

fn msys_root() -> Option<String> {
    env::var_os("MSYSTEM")?;
    let exe = env::var("EXEPATH").ok()?.replace('\\', "/");
    let root = exe.trim_end_matches('/');
    let root = root.strip_suffix("/usr/bin").or_else(|| root.strip_suffix("/bin")).unwrap_or(root);
    Some(root.to_string())
}

fn strip_root(value: &str, prefix: &str) -> Option<String> {
    let head = value.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        Some(format!("/{}", &value[prefix.len()..]))
    } else {
        None
    }
}

fn unmangle(arg: &str, root: &str) -> String {
    if arg.eq_ignore_ascii_case(root) {
        return "/".to_string();
    }
    let prefix = format!("{root}/");
    if let Some(v) = strip_root(arg, &prefix) {
        return v;
    }
    if let Some(i) = arg.find('=') {
        let (key, value) = arg.split_at(i + 1);
        if let Some(v) = strip_root(value, &prefix) {
            return format!("{key}{v}");
        }
    }
    arg.to_string()
}

fn hex(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(H[(b >> 4) as usize] as char);
        s.push(H[(b & 15) as usize] as char);
    }
    s
}

fn main() {
    let raw: Vec<String> = env::args().skip(1).collect();
    let args: Vec<String> = match msys_root() {
        Some(root) => raw.iter().map(|a| unmangle(a, &root)).collect(),
        None => raw,
    };
    let home = home();
    let root = root(&home);
    let first = args.first().map(String::as_str).unwrap_or("");
    if LOCAL.contains(&first) || !ready(&home, &root) {
        python(&home, &root, &args);
    }
    let started = Instant::now();
    let mut stats = false;
    let mut secret: Option<String> = None;
    let mut argv: Vec<&str> = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--stats" {
            stats = true;
        } else if a == "--env" && i + 1 < args.len() {
            match env::var(&args[i + 1]) {
                Ok(v) => secret = Some(v),
                Err(_) => {
                    eprintln!("err: environment variable {} is not set", args[i + 1]);
                    process::exit(2);
                }
            }
            i += 1;
        } else {
            argv.push(a);
        }
        i += 1;
    }
    let token = match fs::read(home.join("key")) {
        Ok(k) if k.len() >= 32 => hex(&k[..32]),
        _ => python(&home, &root, &args),
    };
    let cwd = env::current_dir().map(|p| p.display().to_string()).unwrap_or_default();
    let mut req = String::with_capacity(256);
    req.push_str("L1\0");
    req.push_str(&token);
    req.push('\0');
    req.push_str(&cwd);
    req.push('\0');
    match &secret {
        Some(s) => {
            req.push('+');
            req.push_str(s);
        }
        None => req.push('-'),
    }
    for a in &argv {
        req.push('\0');
        req.push_str(a);
    }
    let reply = exchange(&home, &root, req.as_bytes());
    let text = String::from_utf8_lossy(&reply);
    let mut parts = text.splitn(3, '\0');
    let code: i32 = parts.next().and_then(|c| c.parse().ok()).unwrap_or(1);
    let _image = parts.next();
    let mut out = parts.next().unwrap_or("").to_string();
    if stats {
        out.push_str(&format!("\n[{} ms, ~{} tokens]", started.elapsed().as_millis(), out.len() / 4 + 1));
    }
    let mut so = io::stdout().lock();
    let _ = so.write_all(out.as_bytes());
    let _ = so.write_all(b"\n");
    let _ = so.flush();
    process::exit(code);
}

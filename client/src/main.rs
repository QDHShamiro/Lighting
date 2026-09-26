use std::env;
use std::ffi::{c_void, OsStr};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, RawHandle};
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::ptr::{null, null_mut};
use std::thread::sleep;
use std::time::{Duration, Instant};

const HIDDEN: u32 = 0x0000_0200 | 0x0800_0000;
const BREAKAWAY: u32 = 0x0100_0000;
const EXTENDED_STARTUPINFO: u32 = 0x0008_0000;
const USE_STD_HANDLES: u32 = 0x0000_0100;
const HANDLE_LIST: usize = 0x0002_0002;
const ERROR_PIPE_BUSY: i32 = 231;
const LOCAL: &[&str] = &[
    "", "help", "-h", "--help", "--version", "daemon", "host", "mcp", "setup", "uninstall", "install",
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

#[repr(C)]
struct StartupInfoEx {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    geometry: [u32; 7],
    flags: u32,
    show_window: u16,
    reserved2_size: u16,
    reserved2: *mut u8,
    std_input: RawHandle,
    std_output: RawHandle,
    std_error: RawHandle,
    attributes: *mut c_void,
}

#[repr(C)]
struct ProcessInfo {
    process: RawHandle,
    thread: RawHandle,
    ids: [u32; 2],
}

#[link(name = "kernel32")]
extern "system" {
    fn SetHandleInformation(handle: RawHandle, mask: u32, flags: u32) -> i32;
    fn InitializeProcThreadAttributeList(list: *mut c_void, count: u32, flags: u32, size: *mut usize) -> i32;
    fn UpdateProcThreadAttribute(
        list: *mut c_void,
        flags: u32,
        attribute: usize,
        value: *const c_void,
        size: usize,
        previous: *mut c_void,
        returned: *mut usize,
    ) -> i32;
    fn DeleteProcThreadAttributeList(list: *mut c_void);
    fn CreateProcessW(
        application: *const u16,
        command: *mut u16,
        process_attributes: *const c_void,
        thread_attributes: *const c_void,
        inherit: i32,
        flags: u32,
        environment: *const c_void,
        directory: *const u16,
        startup: *const StartupInfoEx,
        info: *mut ProcessInfo,
    ) -> i32;
    fn CloseHandle(handle: RawHandle) -> i32;
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

fn start_daemon(home: &Path, root: &Path) {
    let _ = fs::create_dir_all(home);
    let Ok(log) = OpenOptions::new().create(true).append(true).open(home.join("daemon.log")) else {
        return;
    };
    let handle = log.as_raw_handle();
    env::set_var("PYTHONPATH", root);
    env::set_var("LIGHTING_ROOT", root);
    let line = format!("\"{}\" -X utf8 -m lighting daemon", scripts(home, "pythonw.exe").display());
    let mut command = wide(OsStr::new(&line));
    let directory = wide(home.as_os_str());
    unsafe {
        let mut size = 0usize;
        InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut size);
        let mut buffer = vec![0usize; size / size_of::<usize>() + 1];
        let list = buffer.as_mut_ptr() as *mut c_void;
        if SetHandleInformation(handle, 1, 1) == 0 || InitializeProcThreadAttributeList(list, 1, 0, &mut size) == 0 {
            return;
        }
        let handles = [handle];
        let listed = UpdateProcThreadAttribute(
            list,
            0,
            HANDLE_LIST,
            handles.as_ptr() as *const c_void,
            size_of::<RawHandle>(),
            null_mut(),
            null_mut(),
        );
        if listed != 0 {
            let mut startup: StartupInfoEx = zeroed();
            startup.cb = size_of::<StartupInfoEx>() as u32;
            startup.flags = USE_STD_HANDLES;
            startup.std_output = handle;
            startup.std_error = handle;
            startup.attributes = list;
            for flags in [HIDDEN | BREAKAWAY, HIDDEN] {
                let mut info: ProcessInfo = zeroed();
                let flags = flags | EXTENDED_STARTUPINFO;
                let ok = CreateProcessW(
                    null(),
                    command.as_mut_ptr(),
                    null(),
                    null(),
                    1,
                    flags,
                    null(),
                    directory.as_ptr(),
                    &startup,
                    &mut info,
                );
                if ok != 0 {
                    CloseHandle(info.process);
                    CloseHandle(info.thread);
                    break;
                }
            }
        }
        DeleteProcThreadAttributeList(list);
    }
}

fn pipe_name() -> String {
    let user = env::var("USERNAME").unwrap_or_else(|_| "user".into()).to_lowercase();
    format!(r"\\.\pipe\lighting-{user}")
}

fn open_pipe() -> io::Result<File> {
    OpenOptions::new().read(true).write(true).open(pipe_name())
}

fn exchange(home: &Path, root: &Path, req: &[u8], idle: Option<&str>) -> Vec<u8> {
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
                    if let Some(text) = idle {
                        return format!("0\0\0{text}").into_bytes();
                    }
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
    let quiet = args.iter().any(|a| a == "--quiet");
    let idle = match first {
        "stop" => Some("lighting daemon not running"),
        "done" if quiet => Some(""),
        "done" => Some("nothing to close"),
        _ => None,
    };
    if LOCAL.contains(&first) || !ready(&home, &root) {
        if first == "done" {
            process::exit(0);
        }
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
        _ if first == "done" => process::exit(0),
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
    let reply = exchange(&home, &root, req.as_bytes(), idle);
    let text = String::from_utf8_lossy(&reply);
    let mut parts = text.splitn(3, '\0');
    let code: i32 = parts.next().and_then(|c| c.parse().ok()).unwrap_or(1);
    let _image = parts.next();
    let mut out = parts.next().unwrap_or("").to_string();
    if stats {
        out.push_str(&format!("\n[{} ms, ~{} tokens]", started.elapsed().as_millis(), out.len() / 4 + 1));
    }
    if !out.is_empty() {
        let mut so = io::stdout().lock();
        let _ = so.write_all(out.as_bytes());
        let _ = so.write_all(b"\n");
        let _ = so.flush();
    }
    process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undoes_git_bash_path_rewrites() {
        let root = "C:/Program Files/Git";
        assert_eq!(unmangle("C:/Program Files/Git/QDHShamiro", root), "/QDHShamiro");
        assert_eq!(unmangle("c:/program files/git", root), "/");
        assert_eq!(unmangle("a=C:/Program Files/Git/b", root), "a=/b");
        assert_eq!(unmangle("C:/Users/x", root), "C:/Users/x");
        assert_eq!(hex(&[0, 171, 255]), "00abff");
    }
}

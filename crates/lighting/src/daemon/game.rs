const GAME_EXES: &[&str] = &[
    "fortniteclient-win64-shipping.exe",
    "rocketleague.exe",
    "cs2.exe",
    "gta5.exe",
    "r5apex.exe",
    "robloxplayerbeta.exe",
    "valorant-win64-shipping.exe",
    "leagueoflegends.exe",
    "overwatch.exe",
    "eldenring.exe",
    "cod.exe",
    "destiny2.exe",
    "dota2.exe",
];
const GAME_SUFFIXES: &[&str] = &["-win64-shipping.exe"];
const GAME_PATHS: &[&str] = &["/steamapps/common/", "/epic games/"];
const GAME_PATH_SKIP: &[&str] = &[
    "/epic games/launcher/",
    "/epic online services/",
    "eosoverlay",
    "epicgameslauncher",
    "webhelper",
    "easyanticheat",
    "battleye",
    "crashreport",
    "launcher",
    "cefsubprocess",
    "redist",
    "/steam/steam.exe",
    "/steam/bin/",
];
const MINECRAFT: &str = "minecraft";

fn file_name(path: &str) -> String {
    path.replace('\\', "/").rsplit('/').next().unwrap_or("").to_string()
}

pub fn is_game(path: &str) -> bool {
    let p = path.to_lowercase().replace('\\', "/");
    if GAME_PATH_SKIP.iter().any(|x| p.contains(x)) {
        return false;
    }
    let name = file_name(&p);
    GAME_EXES.contains(&name.as_str())
        || GAME_SUFFIXES.iter().any(|s| name.ends_with(s))
        || GAME_PATHS.iter().any(|m| p.contains(m))
}

pub fn running() -> String {
    sys::running()
}

#[cfg(windows)]
mod sys {
    use std::os::windows::io::RawHandle;

    use super::{MINECRAFT, file_name, is_game};

    type Handle = isize;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct MonitorInfo {
        size: u32,
        monitor: Rect,
        _work: Rect,
        _flags: u32,
    }

    const MINECRAFT_EXES: &[&str] = &["javaw.exe", "java.exe"];
    const SHELL_CLASSES: &[&str] = &["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];
    const QUERY_LIMITED: u32 = 0x1000;
    const NEAREST: u32 = 2;
    const FULLSCREEN: [i32; 2] = [2, 3];
    const D3D: i32 = 3;
    const MAX_PIDS: usize = 8192;
    const PATH_UNITS: usize = 1024;
    const CLASS_UNITS: usize = 64;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn EnumWindows(visit: extern "system" fn(Handle, isize) -> i32, data: isize) -> i32;
        fn GetWindowTextLengthW(hwnd: Handle) -> i32;
        fn GetWindowTextW(hwnd: Handle, buf: *mut u16, len: i32) -> i32;
        fn IsWindowVisible(hwnd: Handle) -> i32;
        fn GetWindowThreadProcessId(hwnd: Handle, pid: *mut u32) -> u32;
        fn GetForegroundWindow() -> Handle;
        fn GetClassNameW(hwnd: Handle, buf: *mut u16, len: i32) -> i32;
        fn GetWindowRect(hwnd: Handle, rect: *mut Rect) -> i32;
        fn MonitorFromWindow(hwnd: Handle, flags: u32) -> Handle;
        fn GetMonitorInfoW(monitor: Handle, info: *mut MonitorInfo) -> i32;
    }

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHQueryUserNotificationState(state: *mut i32) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> RawHandle;
        fn QueryFullProcessImageNameW(process: RawHandle, flags: u32, buf: *mut u16, size: *mut u32) -> i32;
        fn CloseHandle(handle: RawHandle) -> i32;
        fn K32EnumProcesses(pids: *mut u32, bytes: u32, needed: *mut u32) -> i32;
    }

    fn path_of(pid: u32) -> String {
        let process = unsafe { OpenProcess(QUERY_LIMITED, 0, pid) };
        if process.is_null() {
            return String::new();
        }
        let mut buf = [0u16; PATH_UNITS];
        let mut size = PATH_UNITS as u32;
        let ok = unsafe { QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut size) };
        unsafe { CloseHandle(process) };
        if ok == 0 { String::new() } else { String::from_utf16_lossy(&buf[..(size as usize).min(PATH_UNITS)]) }
    }

    fn exe_of(pid: u32) -> String {
        file_name(&path_of(pid)).to_lowercase()
    }

    fn title(hwnd: Handle) -> String {
        let n = unsafe { GetWindowTextLengthW(hwnd) };
        if n <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; n as usize + 1];
        let got = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
        String::from_utf16_lossy(&buf[..(got.max(0) as usize).min(buf.len())])
    }

    fn pid_of(hwnd: Handle) -> u32 {
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        pid
    }

    extern "system" fn visit(hwnd: Handle, data: isize) -> i32 {
        let found = unsafe { &mut *(data as *mut Vec<u32>) };
        if unsafe { IsWindowVisible(hwnd) } != 0 && title(hwnd).to_lowercase().starts_with(MINECRAFT) {
            found.push(pid_of(hwnd));
        }
        1
    }

    fn covers(hwnd: Handle) -> bool {
        let mut class = [0u16; CLASS_UNITS];
        let n = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), CLASS_UNITS as i32) };
        let class = String::from_utf16_lossy(&class[..(n.max(0) as usize).min(CLASS_UNITS)]);
        let mut rect = Rect::default();
        if SHELL_CLASSES.contains(&class.as_str()) || unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
            return false;
        }
        let mut info = MonitorInfo { size: size_of::<MonitorInfo>() as u32, ..MonitorInfo::default() };
        if unsafe { GetMonitorInfoW(MonitorFromWindow(hwnd, NEAREST), &mut info) } == 0 {
            return false;
        }
        let m = info.monitor;
        rect.left <= m.left && rect.top <= m.top && rect.right >= m.right && rect.bottom >= m.bottom
    }

    fn fullscreen(front: Handle) -> bool {
        let mut state = 0;
        if unsafe { SHQueryUserNotificationState(&mut state) } != 0 {
            return false;
        }
        state == D3D || (FULLSCREEN.contains(&state) && front != 0 && covers(front))
    }

    pub fn running() -> String {
        let mut found: Vec<u32> = Vec::new();
        unsafe { EnumWindows(visit, &mut found as *mut Vec<u32> as isize) };
        if found.iter().any(|&pid| MINECRAFT_EXES.contains(&exe_of(pid).as_str())) {
            return MINECRAFT.into();
        }
        let front = unsafe { GetForegroundWindow() };
        if fullscreen(front) {
            let exe = if front == 0 { String::new() } else { exe_of(pid_of(front)) };
            return format!("fullscreen {exe}").trim().to_string();
        }
        let mut pids = vec![0u32; MAX_PIDS];
        let mut needed = 0u32;
        let bytes = (MAX_PIDS * size_of::<u32>()) as u32;
        if unsafe { K32EnumProcesses(pids.as_mut_ptr(), bytes, &mut needed) } == 0 {
            return String::new();
        }
        let count = (needed as usize / size_of::<u32>()).min(MAX_PIDS);
        pids[..count]
            .iter()
            .map(|&pid| path_of(pid))
            .find(|p| !p.is_empty() && is_game(p))
            .map_or(String::new(), |p| file_name(&p))
    }
}

#[cfg(not(windows))]
mod sys {
    use std::process::Command;

    use super::{MINECRAFT, file_name, is_game};

    pub fn running() -> String {
        let Ok(out) = Command::new("ps").args(["-axo", "args="]).output() else { return String::new() };
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let exe = line.trim().split(' ').next().unwrap_or("");
            if !exe.is_empty() && is_game(exe) {
                return file_name(exe);
            }
            let low = line.to_lowercase();
            let java = exe.to_lowercase().contains("java");
            if java
                && (low.contains("net.minecraft")
                    || low.contains(&format!(".{MINECRAFT}"))
                    || low.contains(&format!("/{MINECRAFT}")))
            {
                return MINECRAFT.into();
            }
        }
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn games_are_told_apart_from_launchers() {
        assert!(is_game(r"C:\Program Files (x86)\Steam\steamapps\common\Hades\Hades.exe"));
        assert!(is_game("D:/Games/FortniteClient-Win64-Shipping.exe"));
        assert!(is_game("C:/Riot/cs2.exe"));
        assert!(!is_game("C:/Program Files (x86)/Epic Games/Launcher/Portal/EpicGamesLauncher.exe"));
        assert!(!is_game("C:/Program Files (x86)/Steam/steamapps/common/Steamworks Shared/_CommonRedist/x.exe"));
        assert!(!is_game("C:/Program Files/Java/bin/javaw.exe"));
        assert_eq!(file_name(r"C:\a\b\Game.exe"), "Game.exe");
    }
}

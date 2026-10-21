import subprocess

from lighting import defaults as D


def is_game(path):
    p = path.lower().replace("\\", "/")
    if any(x in p for x in D.GAME_PATH_SKIP):
        return False
    name = p.rsplit("/", 1)[-1]
    return name in D.GAME_EXES or name.endswith(D.GAME_SUFFIXES) or any(m in p for m in D.GAME_PATHS)


def running():
    try:
        return _windows() if D.WIN else _unix()
    except Exception:
        return ""


def _windows():
    import ctypes
    from ctypes import wintypes as w
    state = ctypes.c_int(0)
    if ctypes.windll.shell32.SHQueryUserNotificationState(ctypes.byref(state)) == 0 and state.value in D.GAME_FULLSCREEN:
        return "fullscreen"
    k32, user32 = ctypes.windll.kernel32, ctypes.windll.user32

    def path_of(pid):
        h = k32.OpenProcess(0x1000, False, pid)
        if not h:
            return ""
        try:
            size, buf = w.DWORD(1024), ctypes.create_unicode_buffer(1024)
            return buf.value if k32.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(size)) else ""
        finally:
            k32.CloseHandle(h)

    title, exes = D.GAME_MINECRAFT
    found = []

    @ctypes.WINFUNCTYPE(w.BOOL, w.HWND, w.LPARAM)
    def visit(hwnd, _):
        n = user32.GetWindowTextLengthW(hwnd)
        if n and user32.IsWindowVisible(hwnd):
            buf = ctypes.create_unicode_buffer(n + 1)
            user32.GetWindowTextW(hwnd, buf, n + 1)
            if buf.value.lower().startswith(title):
                pid = w.DWORD(0)
                user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
                found.append(pid.value)
        return True

    user32.EnumWindows(visit, 0)
    if any(path_of(pid).lower().replace("\\", "/").rsplit("/", 1)[-1] in exes for pid in found):
        return "minecraft"
    pids, need = (w.DWORD * 8192)(), w.DWORD()
    if not ctypes.windll.psapi.EnumProcesses(ctypes.byref(pids), ctypes.sizeof(pids), ctypes.byref(need)):
        return ""
    for pid in pids[:need.value // ctypes.sizeof(w.DWORD)]:
        p = path_of(pid)
        if p and is_game(p):
            return p.replace("\\", "/").rsplit("/", 1)[-1]
    return ""


def _unix():
    out = subprocess.run(["ps", "-axo", "args="], capture_output=True, text=True, timeout=5).stdout
    title, _ = D.GAME_MINECRAFT
    for line in out.splitlines():
        exe = line.strip().split(" ", 1)[0]
        if exe and is_game(exe):
            return exe.rsplit("/", 1)[-1]
        low = line.lower()
        if "java" in exe.lower() and ("net.minecraft" in low or "." + title in low or "/" + title in low):
            return "minecraft"
    return ""

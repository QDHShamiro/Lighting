import subprocess

from lighting import defaults as D

SHELL_CLASSES = {"Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"}


def fullscreen(state, covers):
    return state == D.GAME_D3D or (state in D.GAME_FULLSCREEN and covers())


def front_covers():
    import ctypes
    from ctypes import wintypes as w

    class Info(ctypes.Structure):
        _fields_ = [("cbSize", w.DWORD), ("rcMonitor", w.RECT), ("rcWork", w.RECT), ("dwFlags", w.DWORD)]

    user32 = ctypes.WinDLL("user32")
    user32.GetForegroundWindow.restype = w.HWND
    user32.GetClassNameW.argtypes = [w.HWND, w.LPWSTR, ctypes.c_int]
    user32.GetWindowRect.argtypes = [w.HWND, ctypes.POINTER(w.RECT)]
    user32.MonitorFromWindow.argtypes = [w.HWND, w.DWORD]
    user32.MonitorFromWindow.restype = w.HMONITOR
    user32.GetMonitorInfoW.argtypes = [w.HMONITOR, ctypes.POINTER(Info)]
    hwnd = user32.GetForegroundWindow()
    if not hwnd:
        return False
    cls, rect, info = ctypes.create_unicode_buffer(64), w.RECT(), Info(ctypes.sizeof(Info))
    user32.GetClassNameW(hwnd, cls, 64)
    if cls.value in SHELL_CLASSES or not user32.GetWindowRect(hwnd, ctypes.byref(rect)):
        return False
    if not user32.GetMonitorInfoW(user32.MonitorFromWindow(hwnd, 2), ctypes.byref(info)):
        return False
    m = info.rcMonitor
    return rect.left <= m.left and rect.top <= m.top and rect.right >= m.right and rect.bottom >= m.bottom


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
    if ctypes.windll.shell32.SHQueryUserNotificationState(ctypes.byref(state)) == 0 and fullscreen(state.value, front_covers):
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

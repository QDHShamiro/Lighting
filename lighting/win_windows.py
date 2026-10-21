import contextlib
import ctypes
import ctypes.wintypes as W
import os
import threading
import time

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
dwmapi = ctypes.WinDLL("dwmapi")
advapi32 = ctypes.WinDLL("advapi32", use_last_error=True)

ULONG_PTR = ctypes.c_size_t


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [("dx", W.LONG), ("dy", W.LONG), ("mouseData", W.DWORD), ("dwFlags", W.DWORD),
                ("time", W.DWORD), ("dwExtraInfo", ULONG_PTR)]


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [("wVk", W.WORD), ("wScan", W.WORD), ("dwFlags", W.DWORD), ("time", W.DWORD),
                ("dwExtraInfo", ULONG_PTR)]


class HARDWAREINPUT(ctypes.Structure):
    _fields_ = [("uMsg", W.DWORD), ("wParamL", W.WORD), ("wParamH", W.WORD)]


class _U(ctypes.Union):
    _fields_ = [("mi", MOUSEINPUT), ("ki", KEYBDINPUT), ("hi", HARDWAREINPUT)]


class INPUT(ctypes.Structure):
    _fields_ = [("type", W.DWORD), ("u", _U)]


class LASTINPUTINFO(ctypes.Structure):
    _fields_ = [("cbSize", W.UINT), ("dwTime", W.DWORD)]


class GUITHREADINFO(ctypes.Structure):
    _fields_ = [("cbSize", W.DWORD), ("flags", W.DWORD), ("hwndActive", W.HWND), ("hwndFocus", W.HWND),
                ("hwndCapture", W.HWND), ("hwndMenuOwner", W.HWND), ("hwndMoveSize", W.HWND),
                ("hwndCaret", W.HWND), ("rcCaret", W.RECT)]


WNDENUMPROC = ctypes.WINFUNCTYPE(W.BOOL, W.HWND, W.LPARAM)
user32.EnumWindows.argtypes = [WNDENUMPROC, W.LPARAM]
user32.GetWindowTextW.argtypes = [W.HWND, W.LPWSTR, ctypes.c_int]
user32.GetWindowTextLengthW.argtypes = [W.HWND]
user32.GetClassNameW.argtypes = [W.HWND, W.LPWSTR, ctypes.c_int]
user32.IsWindowVisible.argtypes = [W.HWND]
user32.IsIconic.argtypes = [W.HWND]
user32.GetWindow.argtypes = [W.HWND, W.UINT]
user32.GetWindow.restype = W.HWND
user32.GetForegroundWindow.restype = W.HWND
user32.SetForegroundWindow.argtypes = [W.HWND]
user32.BringWindowToTop.argtypes = [W.HWND]
user32.ShowWindow.argtypes = [W.HWND, ctypes.c_int]
user32.GetWindowThreadProcessId.argtypes = [W.HWND, ctypes.POINTER(W.DWORD)]
user32.GetWindowThreadProcessId.restype = W.DWORD
user32.AttachThreadInput.argtypes = [W.DWORD, W.DWORD, W.BOOL]
user32.GetWindowRect.argtypes = [W.HWND, ctypes.POINTER(W.RECT)]
user32.SetCursorPos.argtypes = [ctypes.c_int, ctypes.c_int]
user32.GetCursorPos.argtypes = [ctypes.POINTER(W.POINT)]
user32.SendInput.argtypes = [W.UINT, ctypes.POINTER(INPUT), ctypes.c_int]
user32.GetLastInputInfo.argtypes = [ctypes.POINTER(LASTINPUTINFO)]
user32.VkKeyScanW.argtypes = [W.WCHAR]
user32.VkKeyScanW.restype = ctypes.c_short
user32.MapVirtualKeyW.argtypes = [W.UINT, W.UINT]
user32.MapVirtualKeyW.restype = W.UINT
user32.PostMessageW.argtypes = [W.HWND, W.UINT, W.WPARAM, W.LPARAM]
user32.GetGUIThreadInfo.argtypes = [W.DWORD, ctypes.POINTER(GUITHREADINFO)]
user32.ChildWindowFromPointEx.argtypes = [W.HWND, W.POINT, W.UINT]
user32.ChildWindowFromPointEx.restype = W.HWND
user32.ScreenToClient.argtypes = [W.HWND, ctypes.POINTER(W.POINT)]
user32.FindWindowExW.argtypes = [W.HWND, W.HWND, W.LPCWSTR, W.LPCWSTR]
user32.FindWindowExW.restype = W.HWND
user32.IsChild.argtypes = [W.HWND, W.HWND]
user32.SetWindowPos.argtypes = [W.HWND, W.HWND, ctypes.c_int, ctypes.c_int, ctypes.c_int, ctypes.c_int, W.UINT]
user32.OpenClipboard.argtypes = [W.HWND]
user32.GetClipboardData.argtypes = [W.UINT]
user32.GetClipboardData.restype = W.HANDLE
user32.SetClipboardData.argtypes = [W.UINT, W.HANDLE]
user32.SetClipboardData.restype = W.HANDLE
user32.GetSystemMetrics.argtypes = [ctypes.c_int]
user32.IsWindow.argtypes = [W.HWND]
kernel32.GlobalAlloc.argtypes = [W.UINT, ctypes.c_size_t]
kernel32.GlobalAlloc.restype = W.HGLOBAL
kernel32.GlobalLock.argtypes = [W.HGLOBAL]
kernel32.GlobalLock.restype = W.LPVOID
kernel32.GlobalUnlock.argtypes = [W.HGLOBAL]
kernel32.GlobalSize.argtypes = [W.HGLOBAL]
kernel32.GlobalSize.restype = ctypes.c_size_t
kernel32.OpenProcess.argtypes = [W.DWORD, W.BOOL, W.DWORD]
kernel32.OpenProcess.restype = W.HANDLE
kernel32.QueryFullProcessImageNameW.argtypes = [W.HANDLE, W.DWORD, W.LPWSTR, ctypes.POINTER(W.DWORD)]
kernel32.CloseHandle.argtypes = [W.HANDLE]
kernel32.GetCurrentThreadId.restype = W.DWORD
kernel32.GetTickCount.restype = W.DWORD
dwmapi.DwmGetWindowAttribute.argtypes = [W.HWND, W.DWORD, ctypes.c_void_p, W.DWORD]
advapi32.OpenProcessToken.argtypes = [W.HANDLE, W.DWORD, ctypes.POINTER(W.HANDLE)]
advapi32.GetTokenInformation.argtypes = [W.HANDLE, ctypes.c_int, ctypes.c_void_p, W.DWORD, ctypes.POINTER(W.DWORD)]

KEYEVENTF_EXTENDEDKEY = 0x0001
KEYEVENTF_KEYUP = 0x0002
KEYEVENTF_UNICODE = 0x0004
KEYEVENTF_SCANCODE = 0x0008
MOUSE = {"left": (0x0002, 0x0004), "right": (0x0008, 0x0010), "middle": (0x0020, 0x0040)}
MOUSEEVENTF_WHEEL = 0x0800
VK = {
    "enter": 0x0D, "return": 0x0D, "tab": 0x09, "esc": 0x1B, "escape": 0x1B, "backspace": 0x08, "space": 0x20,
    "delete": 0x2E, "del": 0x2E, "insert": 0x2D, "home": 0x24, "end": 0x23, "pageup": 0x21, "pagedown": 0x22,
    "left": 0x25, "up": 0x26, "right": 0x27, "down": 0x28, "ctrl": 0x11, "control": 0x11, "shift": 0x10,
    "alt": 0x12, "win": 0x5B, "meta": 0x5B, "capslock": 0x14, "printscreen": 0x2C, "menu": 0x5D, "apps": 0x5D,
    "pause": 0x13, "numlock": 0x90, "scrolllock": 0x91,
    "arrowleft": 0x25, "arrowup": 0x26, "arrowright": 0x27, "arrowdown": 0x28, "pgup": 0x21, "pgdn": 0x22,
    "ins": 0x2D, "prtsc": 0x2C, "bksp": 0x08, "plus": 0xBB, "minus": 0xBD, "comma": 0xBC, "period": 0xBE,
    "playpause": 0xB3, "play": 0xB3, "nexttrack": 0xB0, "prevtrack": 0xB1, "previoustrack": 0xB1, "stop": 0xB2,
    "mediastop": 0xB2, "volumeup": 0xAF, "volup": 0xAF, "volumedown": 0xAE, "voldown": 0xAE, "mute": 0xAD,
    "volumemute": 0xAD, "browserback": 0xA6, "browserforward": 0xA7, "browserrefresh": 0xA8, "browsersearch": 0xAA,
    "browserhome": 0xAC, "numpadmultiply": 0x6A, "numpadadd": 0x6B, "numpadsubtract": 0x6D, "numpaddecimal": 0x6E,
    "numpaddivide": 0x6F, "lctrl": 0xA2, "rctrl": 0xA3, "lshift": 0xA0, "rshift": 0xA1, "lalt": 0xA4, "ralt": 0xA5,
    "altgr": 0xA5, "lwin": 0x5B, "rwin": 0x5C, "super": 0x5B, "cmd": 0x5B, "windows": 0x5B, "option": 0x12,
    "strg": 0x11, "umschalt": 0x10, "entf": 0x2E, "einfg": 0x2D, "pos1": 0x24, "bildauf": 0x21, "bildab": 0x22,
    **{"numpad%d" % i: 0x60 + i for i in range(10)},
}
EXTENDED = {0x2E, 0x2D, 0x24, 0x23, 0x21, 0x22, 0x25, 0x26, 0x27, 0x28, 0x5B, 0x5C, 0x5D, 0x90, 0x2C, 0xA3, 0xA5, 0x6F}
MODS = {"ctrl": 0x11, "control": 0x11, "shift": 0x10, "alt": 0x12, "win": 0x5B, "meta": 0x5B, "strg": 0x11,
        "umschalt": 0x10, "super": 0x5B, "cmd": 0x5B, "windows": 0x5B, "option": 0x12, "altgr": 0xA5, "lctrl": 0xA2,
        "rctrl": 0xA3, "lshift": 0xA0, "rshift": 0xA1, "lalt": 0xA4, "ralt": 0xA5, "lwin": 0x5B, "rwin": 0x5C}
MEDIA = {"playpause", "play", "nexttrack", "prevtrack", "previoustrack", "stop", "mediastop", "volumeup", "volup",
         "volumedown", "voldown", "mute", "volumemute"}


def text_of(hwnd):
    n = user32.GetWindowTextLengthW(hwnd)
    buf = ctypes.create_unicode_buffer(n + 1)
    user32.GetWindowTextW(hwnd, buf, n + 1)
    return buf.value


def class_of(hwnd):
    buf = ctypes.create_unicode_buffer(256)
    user32.GetClassNameW(hwnd, buf, 256)
    return buf.value


def pid_of(hwnd):
    pid = W.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    return pid.value


def exe_of(pid):
    h = kernel32.OpenProcess(0x1000, False, pid)
    if not h:
        return ""
    try:
        buf = ctypes.create_unicode_buffer(1024)
        size = W.DWORD(1024)
        if kernel32.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(size)):
            return os.path.basename(buf.value)
        return ""
    finally:
        kernel32.CloseHandle(h)


def elevated(pid):
    h = kernel32.OpenProcess(0x1000, False, pid)
    if not h:
        return True
    try:
        tok = W.HANDLE()
        if not advapi32.OpenProcessToken(h, 0x0008, ctypes.byref(tok)):
            return True
        try:
            val = W.DWORD()
            size = W.DWORD()
            if advapi32.GetTokenInformation(tok, 20, ctypes.byref(val), 4, ctypes.byref(size)):
                return bool(val.value)
            return False
        finally:
            kernel32.CloseHandle(tok)
    finally:
        kernel32.CloseHandle(h)


def cloaked(hwnd):
    val = ctypes.c_int(0)
    dwmapi.DwmGetWindowAttribute(hwnd, 14, ctypes.byref(val), 4)
    return val.value != 0


def rect(hwnd):
    r = W.RECT()
    if dwmapi.DwmGetWindowAttribute(hwnd, 9, ctypes.byref(r), ctypes.sizeof(r)) != 0:
        user32.GetWindowRect(hwnd, ctypes.byref(r))
    return r.left, r.top, r.right, r.bottom


def windows(owned=False):
    fg = user32.GetForegroundWindow()
    out = []

    def cb(hwnd, _):
        if not user32.IsWindowVisible(hwnd) or (not owned and user32.GetWindow(hwnd, 4)):
            return True
        title = text_of(hwnd)
        if not title or cloaked(hwnd):
            return True
        cls = class_of(hwnd)
        if cls in ("Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"):
            return True
        l, t, r, b = rect(hwnd)
        if r - l < 40 or b - t < 30:
            return True
        pid = pid_of(hwnd)
        out.append({"hwnd": hwnd, "title": title, "cls": cls, "pid": pid, "exe": exe_of(pid),
                    "min": bool(user32.IsIconic(hwnd)), "fg": hwnd == fg})
        return True

    user32.EnumWindows(WNDENUMPROC(cb), 0)
    return out


def alive(hwnd):
    return bool(hwnd) and bool(user32.IsWindow(hwnd))


def visible(hwnd):
    return bool(user32.IsWindowVisible(hwnd))


def close(hwnd):
    user32.PostMessageW(hwnd, 0x0010, 0, 0)


def foreground():
    return user32.GetForegroundWindow()


def set_foreground(hwnd):
    if user32.IsIconic(hwnd):
        user32.ShowWindow(hwnd, 9)
    if user32.GetForegroundWindow() == hwnd:
        return True
    fg = user32.GetForegroundWindow()
    me = kernel32.GetCurrentThreadId()
    them = user32.GetWindowThreadProcessId(fg, None) if fg else 0
    if them and them != me:
        user32.AttachThreadInput(me, them, True)
    try:
        key(0x12, False)
        key(0x12, True)
        user32.BringWindowToTop(hwnd)
        user32.SetForegroundWindow(hwnd)
    finally:
        if them and them != me:
            user32.AttachThreadInput(me, them, False)
    time.sleep(0.05)
    return user32.GetForegroundWindow() == hwnd


def idle_ms():
    info = LASTINPUTINFO(ctypes.sizeof(LASTINPUTINFO), 0)
    user32.GetLastInputInfo(ctypes.byref(info))
    return (kernel32.GetTickCount() - info.dwTime) & 0xFFFFFFFF


def cursor():
    p = W.POINT()
    user32.GetCursorPos(ctypes.byref(p))
    return p.x, p.y


def set_cursor(x, y):
    user32.SetCursorPos(int(x), int(y))


def send(inputs):
    arr = (INPUT * len(inputs))(*inputs)
    return user32.SendInput(len(inputs), arr, ctypes.sizeof(INPUT))


def mouse_input(flags, data=0):
    return INPUT(0, _U(mi=MOUSEINPUT(0, 0, data, flags, 0, 0)))


def click(button="left", count=1):
    down, up = MOUSE[button]
    seq = []
    for _ in range(count):
        seq += [mouse_input(down), mouse_input(up)]
    send(seq)


def mouse_down(button="left"):
    send([mouse_input(MOUSE[button][0])])


def mouse_up(button="left"):
    send([mouse_input(MOUSE[button][1])])


def wheel(delta):
    send([mouse_input(MOUSEEVENTF_WHEEL, ctypes.c_uint32(int(delta)).value)])


def key_input(vk, up, scancode=False):
    flags = KEYEVENTF_KEYUP if up else 0
    scan = 0
    if scancode:
        scan = user32.MapVirtualKeyW(vk, 0)
        flags |= KEYEVENTF_SCANCODE
        vk_send = 0
    else:
        vk_send = vk
    if vk in EXTENDED:
        flags |= KEYEVENTF_EXTENDEDKEY
    return INPUT(1, _U(ki=KEYBDINPUT(vk_send, scan, flags, 0, 0)))


def key(vk, up, scancode=False):
    send([key_input(vk, up, scancode)])


def vk_of(name):
    low = name.lower()
    if low in VK:
        return VK[low], 0
    if len(low) >= 2 and low[0] == "f" and low[1:].isdigit() and 1 <= int(low[1:]) <= 24:
        return 0x6F + int(low[1:]), 0
    if len(name) == 1:
        if name.isalpha() and name.isascii():
            return ord(name.upper()), 0
        if name.isdigit():
            return ord(name), 0
        res = user32.VkKeyScanW(name)
        if res == -1:
            return None, 0
        return res & 0xFF, (res >> 8) & 0xFF
    return None, 0


def combo_parts(combo):
    c = combo.replace(" ", "")
    if c == "+" or c.endswith("++"):
        return [p for p in c[:-2].split("+") if p] + ["+"]
    return [p for p in c.split("+") if p] or [combo]


def press(combo, scancode=False):
    parts = combo_parts(combo)
    mods, main = [], parts[-1]
    for p in parts[:-1]:
        if p.lower() not in MODS:
            raise ValueError("unknown modifier '%s'" % p)
        mods.append(MODS[p.lower()])
    vk, shift_state = vk_of(main)
    if vk is None:
        raise ValueError("unknown key '%s'" % main)
    if getattr(_bg, "hwnd", None):
        return bg_press(_bg.hwnd, combo, mods, vk, main)
    if shift_state & 1 and 0x10 not in mods:
        mods.append(0x10)
    if shift_state & 2 and 0x11 not in mods:
        mods.append(0x11)
    if shift_state & 4 and 0x12 not in mods:
        mods.append(0x12)
    seq = [key_input(m, False, scancode) for m in mods]
    seq += [key_input(vk, False, scancode), key_input(vk, True, scancode)]
    seq += [key_input(m, True, scancode) for m in reversed(mods)]
    send(seq)


def type_unicode(text):
    if getattr(_bg, "hwnd", None):
        return bg_type(_bg.hwnd, text)
    seq = []
    for ch in text:
        code = ord(ch)
        if code > 0xFFFF:
            code -= 0x10000
            units = [0xD800 + (code >> 10), 0xDC00 + (code & 0x3FF)]
        else:
            units = [code]
        for u in units:
            seq.append(INPUT(1, _U(ki=KEYBDINPUT(0, u, KEYEVENTF_UNICODE, 0, 0))))
            seq.append(INPUT(1, _U(ki=KEYBDINPUT(0, u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP, 0, 0))))
    for i in range(0, len(seq), 200):
        send(seq[i:i + 200])


_bg = threading.local()
CHAR_KEYS = {0x0D: "\r", 0x09: "\t", 0x08: "\b", 0x1B: "\x1b", 0x20: " "}
SHIFTS = {0x10, 0xA0, 0xA1}
BG_MOUSE = {"left": (0x201, 0x202, 0x203, 1), "right": (0x204, 0x205, 0x206, 2), "middle": (0x207, 0x208, 0x209, 0x10)}
EDIT_KEYS = {"a": (0x00B1, 0, -1), "c": (0x0301, 0, 0), "v": (0x0302, 0, 0), "x": (0x0300, 0, 0), "z": (0x0304, 0, 0)}
CHROMIUM_HOSTS = {"Chrome_RenderWidgetHostHWND", "Chrome_WidgetWin_1", "Chrome_WidgetWin_0"}
FRONT_HINT = "Lighting does not take the screen from the user; lighting config focus away allows it while they are away"


@contextlib.contextmanager
def keys_to(hwnd):
    _bg.hwnd = hwnd
    try:
        yield
    finally:
        _bg.hwnd = None


def focus_child(hwnd):
    info = GUITHREADINFO()
    info.cbSize = ctypes.sizeof(GUITHREADINFO)
    tid = user32.GetWindowThreadProcessId(hwnd, None)
    if tid and user32.GetGUIThreadInfo(tid, ctypes.byref(info)) and info.hwndFocus and (
            info.hwndFocus == hwnd or user32.IsChild(hwnd, info.hwndFocus)):
        return info.hwndFocus
    return user32.FindWindowExW(hwnd, None, "Chrome_RenderWidgetHostHWND", None) or hwnd


def xy_param(x, y):
    return ((y & 0xFFFF) << 16) | (x & 0xFFFF)


def child_at(hwnd, x, y):
    cur = hwnd
    while True:
        pt = W.POINT(int(x), int(y))
        user32.ScreenToClient(cur, ctypes.byref(pt))
        nxt = user32.ChildWindowFromPointEx(cur, pt, 0x0001 | 0x0004)
        if not nxt or nxt == cur:
            return cur, pt.x, pt.y
        cur = nxt


def bg_key(target, vk, up):
    lp = 1 | (user32.MapVirtualKeyW(vk, 0) << 16) | ((1 << 24) if vk in EXTENDED else 0)
    user32.PostMessageW(target, 0x0101 if up else 0x0100, vk, lp | ((3 << 30) if up else 0))


def bg_press(hwnd, combo, mods, vk, main):
    target = focus_child(hwnd)
    edit = EDIT_KEYS.get(main.lower()) if mods == [0x11] and class_of(target) not in CHROMIUM_HOSTS else None
    if edit:
        user32.PostMessageW(target, edit[0], edit[1], edit[2])
        return
    if [m for m in mods if m not in SHIFTS] or (mods and len(main) != 1):
        raise ValueError("%s needs the window in front, and %s" % (combo, FRONT_HINT))
    ch = (main.upper() if mods else main) if len(main) == 1 else CHAR_KEYS.get(vk)
    bg_key(target, vk, False)
    if ch:
        user32.PostMessageW(target, 0x0102, ord(ch), 1)
    bg_key(target, vk, True)


def bg_type(hwnd, text):
    target = focus_child(hwnd)
    for ch in text.replace("\r\n", "\n").replace("\n", "\r"):
        code = ord(ch)
        units = [0xD800 + ((code - 0x10000) >> 10), 0xDC00 + ((code - 0x10000) & 0x3FF)] if code > 0xFFFF else [code]
        for u in units:
            user32.PostMessageW(target, 0x0102, u, 1)


def bg_click(hwnd, x, y, button="left", count=1):
    target, cx, cy = child_at(hwnd, x, y)
    down, up, double, mk = BG_MOUSE[button]
    lp = xy_param(cx, cy)
    user32.PostMessageW(target, 0x0200, 0, lp)
    for i in range(count):
        user32.PostMessageW(target, double if i else down, mk, lp)
        user32.PostMessageW(target, up, 0, lp)


def bg_drag(hwnd, a, b, steps=12):
    target, ax, ay = child_at(hwnd, *a)
    bx, by = ax + int(b[0] - a[0]), ay + int(b[1] - a[1])
    user32.PostMessageW(target, 0x0201, 1, xy_param(ax, ay))
    for i in range(1, steps + 1):
        user32.PostMessageW(target, 0x0200, 1, xy_param(ax + (bx - ax) * i // steps, ay + (by - ay) * i // steps))
        time.sleep(0.012)
    user32.PostMessageW(target, 0x0202, 0, xy_param(bx, by))


def bg_wheel(hwnd, x, y, delta):
    target = child_at(hwnd, x, y)[0]
    user32.PostMessageW(target, 0x020A, ctypes.c_uint16(int(delta)).value << 16, xy_param(int(x), int(y)))


def select_all_in(hwnd):
    user32.PostMessageW(hwnd, 0x00B1, 0, -1)


def put_behind(hwnd, front):
    user32.SetWindowPos(hwnd, front or 1, 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0010)


def clip_get():
    for _ in range(10):
        if user32.OpenClipboard(None):
            break
        time.sleep(0.03)
    else:
        return None
    try:
        h = user32.GetClipboardData(13)
        if not h:
            return ""
        p = kernel32.GlobalLock(h)
        try:
            return ctypes.wstring_at(p)
        finally:
            kernel32.GlobalUnlock(h)
    finally:
        user32.CloseClipboard()


def clip_set(text):
    data = (text or "") + "\0"
    size = len(data.encode("utf-16-le"))
    for _ in range(10):
        if user32.OpenClipboard(None):
            break
        time.sleep(0.03)
    else:
        return False
    try:
        user32.EmptyClipboard()
        h = kernel32.GlobalAlloc(0x0042, size)
        p = kernel32.GlobalLock(h)
        ctypes.memmove(p, data.encode("utf-16-le"), size)
        kernel32.GlobalUnlock(h)
        user32.SetClipboardData(13, h)
        return True
    finally:
        user32.CloseClipboard()


def post_click(hwnd):
    user32.PostMessageW(hwnd, 0x00F5, 0, 0)


def virtual_screen():
    x, y = user32.GetSystemMetrics(76), user32.GetSystemMetrics(77)
    return x, y, x + user32.GetSystemMetrics(78), y + user32.GetSystemMetrics(79)


gdi32 = ctypes.WinDLL("gdi32")
user32.GetDC.argtypes = [W.HWND]
user32.GetDC.restype = W.HDC
user32.ReleaseDC.argtypes = [W.HWND, W.HDC]
user32.PrintWindow.argtypes = [W.HWND, W.HDC, W.UINT]
gdi32.CreateCompatibleDC.argtypes = [W.HDC]
gdi32.CreateCompatibleDC.restype = W.HDC
gdi32.CreateCompatibleBitmap.argtypes = [W.HDC, ctypes.c_int, ctypes.c_int]
gdi32.CreateCompatibleBitmap.restype = W.HBITMAP
gdi32.SelectObject.argtypes = [W.HDC, W.HGDIOBJ]
gdi32.SelectObject.restype = W.HGDIOBJ
gdi32.DeleteObject.argtypes = [W.HGDIOBJ]
gdi32.DeleteDC.argtypes = [W.HDC]
gdi32.GetDIBits.argtypes = [W.HDC, W.HBITMAP, W.UINT, W.UINT, ctypes.c_void_p, ctypes.c_void_p, W.UINT]


class BITMAPINFOHEADER(ctypes.Structure):
    _fields_ = [("biSize", W.DWORD), ("biWidth", W.LONG), ("biHeight", W.LONG), ("biPlanes", W.WORD),
                ("biBitCount", W.WORD), ("biCompression", W.DWORD), ("biSizeImage", W.DWORD),
                ("biXPelsPerMeter", W.LONG), ("biYPelsPerMeter", W.LONG), ("biClrUsed", W.DWORD),
                ("biClrImportant", W.DWORD)]


def print_window(hwnd):
    from PIL import Image
    wr = W.RECT()
    if user32.IsIconic(hwnd) or not user32.GetWindowRect(hwnd, ctypes.byref(wr)):
        return None
    w, h = wr.right - wr.left, wr.bottom - wr.top
    if w <= 0 or h <= 0:
        return None
    screen = user32.GetDC(None)
    dc = gdi32.CreateCompatibleDC(screen)
    bmp = gdi32.CreateCompatibleBitmap(screen, w, h)
    old = gdi32.SelectObject(dc, bmp)
    try:
        if not user32.PrintWindow(hwnd, dc, 2):
            return None
        head = BITMAPINFOHEADER(ctypes.sizeof(BITMAPINFOHEADER), w, -h, 1, 32, 0, 0, 0, 0, 0, 0)
        buf = ctypes.create_string_buffer(w * h * 4)
        if gdi32.GetDIBits(dc, bmp, 0, h, buf, ctypes.byref(head), 0) != h:
            return None
        img = Image.frombuffer("RGB", (w, h), buf, "raw", "BGRX", 0, 1)
    finally:
        gdi32.SelectObject(dc, old)
        gdi32.DeleteObject(bmp)
        gdi32.DeleteDC(dc)
        user32.ReleaseDC(None, screen)
    l, t, r, b = rect(hwnd)
    img = img.crop((l - wr.left, t - wr.top, r - wr.left, b - wr.top))
    return None if max(img.resize((32, 32)).convert("L").getextrema()) < 8 else img


MOD = "ctrl"
SELECT_ALL = "ctrl+a"
PASTE = "ctrl+v"


def minimized(hwnd):
    return bool(user32.IsIconic(hwnd))


def restore_cycle(hwnd):
    user32.ShowWindow(hwnd, 6)
    time.sleep(0.05)
    user32.ShowWindow(hwnd, 9)


def app_list():
    import json
    import subprocess
    script = ("[Console]::OutputEncoding=[Text.Encoding]::UTF8; "
              "Get-StartApps | Select-Object Name,AppID | ConvertTo-Json -Compress")
    res = subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", script],
                         capture_output=True, timeout=30, creationflags=0x08000000)
    try:
        data = json.loads(res.stdout.decode("utf-8", "replace") or "[]")
    except ValueError:
        data = []
    data = data if isinstance(data, list) else [data]
    return [(a.get("Name") or "", a.get("AppID") or "") for a in data if a.get("AppID")]


def open_target(target):
    os.startfile(target, show_cmd=4)


def start_app(app):
    os.startfile("shell:AppsFolder\\" + app[1], show_cmd=4)


def screen_grab(box):
    from PIL import ImageGrab
    return ImageGrab.grab(bbox=box, all_screens=True)


def video_input(x0, y0, size, fps):
    return ["-f", "gdigrab", "-framerate", str(fps), "-offset_x", str(x0), "-offset_y", str(y0), "-video_size", size,
            "-i", "desktop"]

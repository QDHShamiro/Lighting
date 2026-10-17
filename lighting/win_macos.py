import ctypes
import os
import shutil
import subprocess
import threading
import time

import ApplicationServices as AS
import Quartz
from AppKit import NSPasteboard, NSPasteboardTypeString, NSRunningApplication, NSScreen, NSWorkspace

user32 = None
MOD = "cmd"
SELECT_ALL = "cmd+a"
PASTE = "cmd+v"
PASTE_TERMINAL = "cmd+v"
CHROMIUM_CLASS = "Chrome_WidgetWin_1"
TERMINAL_APPS = {"terminal", "iterm2", "ghostty", "wezterm-gui", "wezterm", "alacritty", "kitty", "warp", "hyper", "tabby"}
CHROMIUM_APPS = {"brave browser", "google chrome", "microsoft edge", "chromium", "opera", "vivaldi", "arc"}
SKIP_OWNERS = {"window server", "dock", "control center", "notification center", "systemuiserver", "spotlight",
               "loginwindow", "screencaptureui", "textinputmenuagent", "wallpaper"}

KEYCODES = {
    "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9, "b": 11, "q": 12, "w": 13, "e": 14,
    "r": 15, "y": 16, "t": 17, "1": 18, "2": 19, "3": 20, "4": 21, "6": 22, "5": 23, "=": 24, "9": 25, "7": 26, "-": 27,
    "8": 28, "0": 29, "]": 30, "o": 31, "u": 32, "[": 33, "i": 34, "p": 35, "l": 37, "j": 38, "'": 39, "k": 40, ";": 41,
    "\\": 42, ",": 43, "/": 44, "n": 45, "m": 46, ".": 47, "`": 50,
    "enter": 36, "return": 36, "tab": 48, "space": 49, "backspace": 51, "bksp": 51, "esc": 53, "escape": 53,
    "delete": 117, "del": 117, "entf": 117, "home": 115, "pos1": 115, "end": 119, "pageup": 116, "pgup": 116,
    "bildauf": 116, "pagedown": 121, "pgdn": 121, "bildab": 121, "left": 123, "right": 124, "down": 125, "up": 126,
    "arrowleft": 123, "arrowright": 124, "arrowdown": 125, "arrowup": 126, "capslock": 57, "help": 114, "ins": 114,
    "insert": 114, "einfg": 114, "plus": 24, "minus": 27, "comma": 43, "period": 47,
    "f1": 122, "f2": 120, "f3": 99, "f4": 118, "f5": 96, "f6": 97, "f7": 98, "f8": 100, "f9": 101, "f10": 109,
    "f11": 103, "f12": 111, "f13": 105, "f14": 107, "f15": 113, "f16": 106, "f17": 64, "f18": 79, "f19": 80, "f20": 90,
    "numpad0": 82, "numpad1": 83, "numpad2": 84, "numpad3": 85, "numpad4": 86, "numpad5": 87, "numpad6": 88,
    "numpad7": 89, "numpad8": 91, "numpad9": 92, "numpadmultiply": 67, "numpadadd": 69, "numpadsubtract": 78,
    "numpaddecimal": 65, "numpaddivide": 75,
}
FLAGS = {"cmd": Quartz.kCGEventFlagMaskCommand, "command": Quartz.kCGEventFlagMaskCommand,
         "win": Quartz.kCGEventFlagMaskCommand, "meta": Quartz.kCGEventFlagMaskCommand,
         "super": Quartz.kCGEventFlagMaskCommand, "windows": Quartz.kCGEventFlagMaskCommand,
         "ctrl": Quartz.kCGEventFlagMaskControl, "control": Quartz.kCGEventFlagMaskControl,
         "strg": Quartz.kCGEventFlagMaskControl, "shift": Quartz.kCGEventFlagMaskShift,
         "umschalt": Quartz.kCGEventFlagMaskShift, "alt": Quartz.kCGEventFlagMaskAlternate,
         "option": Quartz.kCGEventFlagMaskAlternate, "opt": Quartz.kCGEventFlagMaskAlternate,
         "altgr": Quartz.kCGEventFlagMaskAlternate, "fn": Quartz.kCGEventFlagMaskSecondaryFn}
MOD_KEYS = {Quartz.kCGEventFlagMaskCommand: 55, Quartz.kCGEventFlagMaskControl: 59, Quartz.kCGEventFlagMaskShift: 56,
            Quartz.kCGEventFlagMaskAlternate: 58, Quartz.kCGEventFlagMaskSecondaryFn: 63}
MEDIA_KEYS = {"playpause": 16, "play": 16, "nexttrack": 17, "prevtrack": 18, "previoustrack": 18, "volumeup": 0,
              "volup": 0, "volumedown": 1, "voldown": 1, "mute": 7, "volumemute": 7, "stop": 16, "mediastop": 16}
MEDIA = set(MEDIA_KEYS)
MOUSE = {"left": (Quartz.kCGEventLeftMouseDown, Quartz.kCGEventLeftMouseUp, Quartz.kCGMouseButtonLeft),
         "right": (Quartz.kCGEventRightMouseDown, Quartz.kCGEventRightMouseUp, Quartz.kCGMouseButtonRight),
         "middle": (Quartz.kCGEventOtherMouseDown, Quartz.kCGEventOtherMouseUp, Quartz.kCGMouseButtonCenter)}

_ax = ctypes.cdll.LoadLibrary("/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices")
_ax._AXUIElementGetWindow.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_uint32)]
_ax._AXUIElementGetWindow.restype = ctypes.c_int32
_cache = {"t": 0.0, "wins": {}, "ax": {}}
_lock = threading.RLock()


def trusted():
    return bool(AS.AXIsProcessTrusted())


def ax_get(el, attr):
    try:
        err, value = AS.AXUIElementCopyAttributeValue(el, attr, None)
    except Exception:
        return None
    return value if err == 0 else None


def ax_set(el, attr, value):
    try:
        return AS.AXUIElementSetAttributeValue(el, attr, value) == 0
    except Exception:
        return False


def ax_do(el, action):
    try:
        return AS.AXUIElementPerformAction(el, action) == 0
    except Exception:
        return False


def ax_window_id(el):
    out = ctypes.c_uint32(0)
    try:
        ptr = el.__c_void_p__()
    except Exception:
        return 0
    return out.value if _ax._AXUIElementGetWindow(ptr, ctypes.byref(out)) == 0 else 0


def point_of(value):
    if value is None:
        return None
    ok, p = AS.AXValueGetValue(value, AS.kAXValueCGPointType, None)
    return (p.x, p.y) if ok else None


def size_of(value):
    if value is None:
        return None
    ok, s = AS.AXValueGetValue(value, AS.kAXValueCGSizeType, None)
    return (s.width, s.height) if ok else None


def app_name(pid):
    app = NSRunningApplication.runningApplicationWithProcessIdentifier_(pid)
    if app is None:
        return ""
    return str(app.localizedName() or "")


def app_path(pid):
    app = NSRunningApplication.runningApplicationWithProcessIdentifier_(pid)
    url = app.bundleURL() if app is not None else None
    return str(url.path()) if url is not None else ""


_electron = {}


def chromium_based(pid):
    if pid in _electron:
        return _electron[pid]
    path = app_path(pid)
    name = app_name(pid).lower()
    hit = name in CHROMIUM_APPS or any(os.path.isdir(os.path.join(path, "Contents", "Frameworks", f)) for f in
                                       ("Electron Framework.framework", "Chromium Embedded Framework.framework"))
    _electron[pid] = hit
    return hit


def ax_windows(pid):
    app = AS.AXUIElementCreateApplication(pid)
    out = {}
    for el in ax_get(app, "AXWindows") or []:
        wid = ax_window_id(el)
        if wid:
            out[wid] = el
    return out


def cg_list(option=None, wid=0):
    option = Quartz.kCGWindowListOptionAll if option is None else option
    return Quartz.CGWindowListCopyWindowInfo(option | Quartz.kCGWindowListExcludeDesktopElements, wid) or []


def refresh(force=False):
    with _lock:
        if not force and time.time() - _cache["t"] < 0.25:
            return _cache["wins"]
        infos = [i for i in cg_list() if i.get("kCGWindowLayer", 1) == 0]
        pids = {}
        for i in infos:
            pids.setdefault(int(i["kCGWindowOwnerPID"]), []).append(i)
        wins, axmap = {}, {}
        for pid, rows in pids.items():
            name = str(rows[0].get("kCGWindowOwnerName") or app_name(pid))
            if name.lower() in SKIP_OWNERS:
                continue
            ax = ax_windows(pid)
            for i in rows:
                wid = int(i["kCGWindowNumber"])
                el = ax.get(wid)
                if el is None:
                    continue
                b = i.get("kCGWindowBounds") or {}
                x, y, w, h = int(b.get("X", 0)), int(b.get("Y", 0)), int(b.get("Width", 0)), int(b.get("Height", 0))
                title = str(ax_get(el, "AXTitle") or i.get("kCGWindowName") or "")
                wins[wid] = {"hwnd": wid, "pid": pid, "exe": name, "title": title or name,
                             "rect": (x, y, x + w, y + h), "onscreen": bool(i.get("kCGWindowIsOnscreen")),
                             "min": bool(ax_get(el, "AXMinimized")), "subrole": str(ax_get(el, "AXSubrole") or "")}
                axmap[wid] = el
        _cache.update(t=time.time(), wins=wins, ax=axmap)
        return wins


def info(hwnd):
    w = refresh().get(hwnd)
    if w is None and hwnd:
        w = refresh(True).get(hwnd)
    return w


def ax_of(hwnd):
    info(hwnd)
    return _cache["ax"].get(hwnd)


def front_pid():
    app = NSWorkspace.sharedWorkspace().frontmostApplication()
    return int(app.processIdentifier()) if app is not None else 0


def foreground():
    pid = front_pid()
    if not pid:
        return 0
    el = ax_get(AS.AXUIElementCreateApplication(pid), "AXFocusedWindow")
    wid = ax_window_id(el) if el is not None else 0
    if wid:
        return wid
    for i in cg_list(Quartz.kCGWindowListOptionOnScreenOnly):
        if int(i.get("kCGWindowOwnerPID", 0)) == pid and i.get("kCGWindowLayer", 1) == 0:
            return int(i["kCGWindowNumber"])
    return 0


def windows(owned=False):
    wins = refresh(True)
    fg = foreground()
    order = [int(i["kCGWindowNumber"]) for i in cg_list(Quartz.kCGWindowListOptionOnScreenOnly)]
    rank = {wid: n for n, wid in enumerate(order)}
    out = []
    for wid, w in sorted(wins.items(), key=lambda kv: rank.get(kv[0], 10 ** 6)):
        l, t, r, b = w["rect"]
        if (r - l < 40 or b - t < 30) and not w["min"]:
            continue
        if not owned and w["subrole"] in ("AXSystemDialog",):
            continue
        out.append({"hwnd": wid, "title": w["title"], "cls": class_of(wid), "pid": w["pid"], "exe": w["exe"],
                    "min": w["min"], "fg": wid == fg})
    return out


def text_of(hwnd):
    el = ax_of(hwnd)
    title = ax_get(el, "AXTitle") if el is not None else None
    if title:
        return str(title)
    w = info(hwnd)
    return w["title"] if w else ""


def class_of(hwnd):
    w = info(hwnd)
    if not w:
        return ""
    name = w["exe"].lower()
    if name in TERMINAL_APPS:
        return "terminal"
    if chromium_based(w["pid"]):
        return CHROMIUM_CLASS
    return w["subrole"] or "AXWindow"


def pid_of(hwnd):
    w = info(hwnd)
    return w["pid"] if w else 0


def exe_of(pid):
    return app_name(pid) if pid else ""


def elevated(pid):
    return False


def rect(hwnd):
    el = ax_of(hwnd)
    if el is not None:
        p, s = point_of(ax_get(el, "AXPosition")), size_of(ax_get(el, "AXSize"))
        if p and s:
            return int(p[0]), int(p[1]), int(p[0] + s[0]), int(p[1] + s[1])
    w = info(hwnd)
    return w["rect"] if w else (0, 0, 0, 0)


def alive(hwnd):
    if not hwnd:
        return False
    rows = cg_list(Quartz.kCGWindowListOptionIncludingWindow, int(hwnd))
    return any(int(i.get("kCGWindowNumber", 0)) == int(hwnd) for i in rows)


def visible(hwnd):
    rows = cg_list(Quartz.kCGWindowListOptionIncludingWindow, int(hwnd))
    return any(int(i.get("kCGWindowNumber", 0)) == int(hwnd) and i.get("kCGWindowIsOnscreen") for i in rows)


def minimized(hwnd):
    el = ax_of(hwnd)
    return bool(ax_get(el, "AXMinimized")) if el is not None else False


def close(hwnd):
    el = ax_of(hwnd)
    btn = ax_get(el, "AXCloseButton") if el is not None else None
    if btn is not None and ax_do(btn, "AXPress"):
        return
    if set_foreground(hwnd):
        press("cmd+w")


def activate(pid):
    app = NSRunningApplication.runningApplicationWithProcessIdentifier_(pid)
    if app is None:
        return False
    return bool(app.activateWithOptions_(1 << 1))


def set_foreground(hwnd):
    w = info(hwnd)
    if not w:
        return False
    el = ax_of(hwnd)
    if el is not None and ax_get(el, "AXMinimized"):
        ax_set(el, "AXMinimized", False)
    activate(w["pid"])
    if el is not None:
        ax_do(el, "AXRaise")
        ax_set(el, "AXMain", True)
    deadline = time.time() + 0.6
    while time.time() < deadline:
        if foreground() == hwnd:
            return True
        time.sleep(0.03)
    return foreground() == hwnd


def restore_cycle(hwnd):
    el = ax_of(hwnd)
    if el is not None:
        ax_set(el, "AXMinimized", False)
        ax_do(el, "AXRaise")


def idle_ms():
    secs = Quartz.CGEventSourceSecondsSinceLastEventType(Quartz.kCGEventSourceStateHIDSystemState, Quartz.kCGAnyInputEventType)
    return int(secs * 1000)


def cursor():
    p = Quartz.CGEventGetLocation(Quartz.CGEventCreate(None))
    return int(p.x), int(p.y)


def set_cursor(x, y):
    Quartz.CGWarpMouseCursorPosition((float(x), float(y)))
    Quartz.CGAssociateMouseAndMouseCursorPosition(True)


def post(ev):
    Quartz.CGEventPost(Quartz.kCGHIDEventTap, ev)


def mouse_event(kind, button, pos=None, clicks=1):
    pos = pos or cursor()
    ev = Quartz.CGEventCreateMouseEvent(None, kind, (float(pos[0]), float(pos[1])), button)
    Quartz.CGEventSetIntegerValueField(ev, Quartz.kCGMouseEventClickState, clicks)
    return ev


def click(button="left", count=1):
    down, up, btn = MOUSE[button]
    for n in range(1, count + 1):
        post(mouse_event(down, btn, clicks=n))
        post(mouse_event(up, btn, clicks=n))


def mouse_down(button="left"):
    down, _, btn = MOUSE[button]
    post(mouse_event(down, btn))


def mouse_up(button="left"):
    _, up, btn = MOUSE[button]
    post(mouse_event(up, btn))


def drag_to(x, y, button="left"):
    kind = {"left": Quartz.kCGEventLeftMouseDragged, "right": Quartz.kCGEventRightMouseDragged}.get(button, Quartz.kCGEventOtherMouseDragged)
    post(mouse_event(kind, MOUSE[button][2], (x, y)))


def wheel(delta):
    lines = int(round(delta / 120.0 * 3)) or (1 if delta > 0 else -1)
    post(Quartz.CGEventCreateScrollWheelEvent(None, Quartz.kCGScrollEventUnitLine, 1, lines))


def combo_parts(combo):
    parts = [p for p in combo.replace(" ", "").split("+") if p] if combo != "+" else ["+"]
    if combo.endswith("++"):
        parts = parts + ["+"]
    return parts


def key_code(name):
    n = name.lower()
    if n in KEYCODES:
        return KEYCODES[n]
    if len(name) == 1 and name.lower() in KEYCODES:
        return KEYCODES[name.lower()]
    raise ValueError("unknown key '%s'" % name)


def key_event(code, down, flags=0):
    ev = Quartz.CGEventCreateKeyboardEvent(None, code, down)
    if flags:
        Quartz.CGEventSetFlags(ev, flags)
    return ev


def media(name):
    from AppKit import NSEvent
    key = MEDIA_KEYS[name]
    for state in (0xA, 0xB):
        ev = NSEvent.otherEventWithType_location_modifierFlags_timestamp_windowNumber_context_subtype_data1_data2_(
            14, (0, 0), state << 8, 0, 0, None, 8, (key << 16) | (state << 8), -1)
        post(ev.CGEvent())


def press(combo, scancode=False):
    parts = combo_parts(combo)
    if len(parts) == 1 and parts[0].lower() in MEDIA_KEYS:
        media(parts[0].lower())
        return
    *mods, last = parts
    flags = 0
    for m in mods:
        f = FLAGS.get(m.lower())
        if f is None:
            raise ValueError("unknown modifier '%s'" % m)
        flags |= f
    if len(last) == 1 and last.isupper() and last.lower() in KEYCODES:
        flags |= Quartz.kCGEventFlagMaskShift
    code = key_code(last)
    held = [k for f, k in MOD_KEYS.items() if flags & f]
    acc = 0
    for f, k in MOD_KEYS.items():
        if flags & f:
            acc |= f
            post(key_event(k, True, acc))
    post(key_event(code, True, flags))
    post(key_event(code, False, flags))
    for f, k in reversed(list(MOD_KEYS.items())):
        if flags & f:
            acc &= ~f
            post(key_event(k, False, acc))
    time.sleep(0.005 * (1 + len(held)))


def type_unicode(text):
    for i in range(0, len(text), 16):
        chunk = text[i:i + 16]
        for down in (True, False):
            ev = Quartz.CGEventCreateKeyboardEvent(None, 0, down)
            Quartz.CGEventKeyboardSetUnicodeString(ev, len(chunk.encode("utf-16-le")) // 2, chunk)
            post(ev)
        time.sleep(0.01)


def clip_get():
    pb = NSPasteboard.generalPasteboard()
    value = pb.stringForType_(NSPasteboardTypeString)
    return str(value) if value is not None else ""


def clip_set(text):
    pb = NSPasteboard.generalPasteboard()
    pb.clearContents()
    return bool(pb.setString_forType_(text, NSPasteboardTypeString))


def post_click(hwnd):
    return False


def displays():
    err, ids, n = Quartz.CGGetActiveDisplayList(16, None, None)
    out = []
    for d in (ids or [])[:n]:
        b = Quartz.CGDisplayBounds(d)
        out.append((int(b.origin.x), int(b.origin.y), int(b.origin.x + b.size.width), int(b.origin.y + b.size.height)))
    return out or [(0, 0, 1440, 900)]


def virtual_screen():
    ds = displays()
    return min(d[0] for d in ds), min(d[1] for d in ds), max(d[2] for d in ds), max(d[3] for d in ds)


def cg_to_pil(img):
    from PIL import Image
    if img is None:
        return None
    w, h = Quartz.CGImageGetWidth(img), Quartz.CGImageGetHeight(img)
    if not w or not h:
        return None
    row = Quartz.CGImageGetBytesPerRow(img)
    data = Quartz.CGDataProviderCopyData(Quartz.CGImageGetDataProvider(img))
    return Image.frombuffer("RGBA", (w, h), bytes(data), "raw", "BGRA", row, 1).convert("RGB")


def to_points(img, box):
    if img is None:
        return None
    w, h = max(1, box[2] - box[0]), max(1, box[3] - box[1])
    return img if img.size == (w, h) else img.resize((w, h))


def print_window(hwnd):
    opts = Quartz.kCGWindowImageBoundsIgnoreFraming | Quartz.kCGWindowImageNominalResolution
    img = Quartz.CGWindowListCreateImage(Quartz.CGRectNull, Quartz.kCGWindowListOptionIncludingWindow, int(hwnd), opts)
    pil = cg_to_pil(img)
    if pil is None or pil.getbbox() is None:
        return None
    return to_points(pil, rect(hwnd))


def screen_grab(box):
    l, t, r, b = box
    rect_ = Quartz.CGRectMake(l, t, r - l, b - t)
    img = Quartz.CGWindowListCreateImage(rect_, Quartz.kCGWindowListOptionOnScreenOnly, Quartz.kCGNullWindowID,
                                         Quartz.kCGWindowImageNominalResolution)
    pil = cg_to_pil(img)
    if pil is None:
        from PIL import ImageGrab
        pil = ImageGrab.grab(bbox=box)
    return to_points(pil, box)


def scale():
    s = NSScreen.mainScreen()
    return float(s.backingScaleFactor()) if s is not None else 1.0


def video_input(x0, y0, size, fps):
    k = scale()
    w, h = (int(v) for v in size.split("x"))
    crop = "crop=%d:%d:%d:%d" % (int(w * k) // 2 * 2, int(h * k) // 2 * 2, int(x0 * k), int(y0 * k))
    return ["-f", "avfoundation", "-capture_cursor", "1", "-framerate", str(fps), "-i", "Capture screen 0:none",
            "-vf", crop]


def app_dirs():
    home = os.path.expanduser("~")
    return ["/Applications", "/Applications/Utilities", "/System/Applications", "/System/Applications/Utilities",
            os.path.join(home, "Applications")]


def app_list():
    out, seen = [], set()
    for d in app_dirs():
        try:
            names = os.listdir(d)
        except OSError:
            continue
        for n in names:
            if n.endswith(".app") and n not in seen:
                seen.add(n)
                out.append((n[:-4], os.path.join(d, n)))
    return out


def run_name(name):
    return None


def open_target(target):
    subprocess.Popen(["open", target], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def start_app(app):
    subprocess.Popen(["open", "-a", app[1]], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def file_dialog(browser_hwnd, before):
    el = ax_of(browser_hwnd)
    if el is not None:
        for kid in ax_get(el, "AXChildren") or []:
            if str(ax_get(kid, "AXRole") or "") == "AXSheet":
                return kid
    w = info(browser_hwnd)
    if not w:
        return None
    for wid, other in refresh(True).items():
        if wid not in before and other["pid"] == w["pid"] and other["subrole"] in ("AXDialog", "AXStandardWindow"):
            return _cache["ax"].get(wid)
    return None


def choose_folder(dlg, path):
    press("cmd+shift+g")
    time.sleep(0.6)
    clip_set(path)
    press("cmd+a")
    press("cmd+v")
    time.sleep(0.3)
    press("enter")
    time.sleep(0.8)
    btn = ax_get(dlg, "AXDefaultButton")
    if btn is not None and ax_do(btn, "AXPress"):
        return True
    press("enter")
    return True


def hotkey_loop(daemon):
    want = Quartz.kCGEventFlagMaskControl | Quartz.kCGEventFlagMaskAlternate

    def cb(proxy, kind, ev, ref):
        if kind == Quartz.kCGEventKeyDown:
            code = Quartz.CGEventGetIntegerValueField(ev, Quartz.kCGKeyboardEventKeycode)
            if code == KEYCODES["end"] and (Quartz.CGEventGetFlags(ev) & want) == want:
                daemon.abort_all()
        return ev

    tap = Quartz.CGEventTapCreate(Quartz.kCGSessionEventTap, Quartz.kCGHeadInsertEventTap,
                                  Quartz.kCGEventTapOptionListenOnly, Quartz.CGEventMaskBit(Quartz.kCGEventKeyDown), cb, None)
    if tap is None:
        return
    daemon.hotkey = True
    src = Quartz.CFMachPortCreateRunLoopSource(None, tap, 0)
    loop = Quartz.CFRunLoopGetCurrent()
    Quartz.CFRunLoopAddSource(loop, src, Quartz.kCFRunLoopCommonModes)
    Quartz.CGEventTapEnable(tap, True)
    Quartz.CFRunLoopRun()


def terminal_app():
    for name in ("iTerm", "Terminal"):
        if os.path.isdir("/Applications/%s.app" % name) or os.path.isdir("/System/Applications/Utilities/%s.app" % name):
            return name
    return "Terminal"


def which(name):
    return shutil.which(name)



KEYNAMES = {v: k for k, v in reversed(list(KEYCODES.items()))}


def window_at(x, y):
    for i in cg_list(Quartz.kCGWindowListOptionOnScreenOnly):
        if i.get("kCGWindowLayer", 1) != 0:
            continue
        b = i.get("kCGWindowBounds") or {}
        if b.get("X", 0) <= x < b.get("X", 0) + b.get("Width", 0) and b.get("Y", 0) <= y < b.get("Y", 0) + b.get("Height", 0):
            return int(i["kCGWindowNumber"])
    return 0


def listen_input(q, stop, ready, injected=False):
    mask = (Quartz.CGEventMaskBit(Quartz.kCGEventLeftMouseDown) | Quartz.CGEventMaskBit(Quartz.kCGEventRightMouseDown)
            | Quartz.CGEventMaskBit(Quartz.kCGEventKeyDown))

    def cb(proxy, kind, ev, ref):
        if not injected and Quartz.CGEventGetIntegerValueField(ev, Quartz.kCGEventSourceStateID) != Quartz.kCGEventSourceStateHIDSystemState:
            return ev
        t = time.time()
        if kind in (Quartz.kCGEventLeftMouseDown, Quartz.kCGEventRightMouseDown):
            p = Quartz.CGEventGetLocation(ev)
            q.put(("m", int(p.x), int(p.y), kind == Quartz.kCGEventRightMouseDown, t))
        elif kind == Quartz.kCGEventKeyDown:
            code = Quartz.CGEventGetIntegerValueField(ev, Quartz.kCGKeyboardEventKeycode)
            flags = Quartz.CGEventGetFlags(ev)
            mods = [n for f, n in ((Quartz.kCGEventFlagMaskControl, "ctrl"), (Quartz.kCGEventFlagMaskAlternate, "alt"),
                                   (Quartz.kCGEventFlagMaskShift, "shift"), (Quartz.kCGEventFlagMaskCommand, "cmd")) if flags & f]
            n, chars = Quartz.CGEventKeyboardGetUnicodeString(ev, 4, None, None)
            ch = chars[:n] if n and chars else ""
            q.put(("k", KEYNAMES.get(code, ""), ch, mods, t))
        return ev

    tap = Quartz.CGEventTapCreate(Quartz.kCGSessionEventTap, Quartz.kCGHeadInsertEventTap,
                                  Quartz.kCGEventTapOptionListenOnly, mask, cb, None)
    ready.set()
    if tap is None:
        return
    src = Quartz.CFMachPortCreateRunLoopSource(None, tap, 0)
    loop = Quartz.CFRunLoopGetCurrent()
    Quartz.CFRunLoopAddSource(loop, src, Quartz.kCFRunLoopCommonModes)
    Quartz.CGEventTapEnable(tap, True)
    while not stop.is_set():
        Quartz.CFRunLoopRunInMode(Quartz.kCFRunLoopDefaultMode, 0.25, False)
    Quartz.CGEventTapEnable(tap, False)


def access(prompt=False):
    if prompt:
        try:
            AS.AXIsProcessTrustedWithOptions({"AXTrustedCheckOptionPrompt": True})
        except Exception:
            pass
        try:
            Quartz.CGRequestScreenCaptureAccess()
        except Exception:
            pass
    try:
        screen = bool(Quartz.CGPreflightScreenCaptureAccess())
    except Exception:
        screen = True
    return {"accessibility": trusted(), "screen recording": screen}

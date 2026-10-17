import os
import shutil
import subprocess
import threading
import time

user32 = None
MOD = "ctrl"
SELECT_ALL = "ctrl+a"
PASTE = "ctrl+v"
PASTE_TERMINAL = "ctrl+shift+v"
CHROMIUM_CLASS = "Chrome_WidgetWin_1"
TERMINAL_APPS = {"gnome-terminal-server", "gnome-terminal", "konsole", "xterm", "tilix", "terminator", "ptyxis",
                 "kitty", "alacritty", "wezterm-gui", "xfce4-terminal", "foot", "ghostty", "warp", "terminal"}
CHROMIUM_APPS = {"brave", "chrome", "msedge", "chromium", "chromium-browser", "opera", "vivaldi", "vivaldi-bin"}
WAYLAND = os.environ.get("XDG_SESSION_TYPE") == "wayland" or (bool(os.environ.get("WAYLAND_DISPLAY")) and not os.environ.get("DISPLAY"))
MEDIA = {"playpause", "play", "nexttrack", "prevtrack", "previoustrack", "stop", "mediastop", "volumeup", "volup",
         "volumedown", "voldown", "mute", "volumemute"}
KEYSYMS = {"enter": "Return", "return": "Return", "tab": "Tab", "space": "space", "backspace": "BackSpace",
           "bksp": "BackSpace", "esc": "Escape", "escape": "Escape", "delete": "Delete", "del": "Delete", "entf": "Delete",
           "insert": "Insert", "ins": "Insert", "einfg": "Insert", "home": "Home", "pos1": "Home", "end": "End",
           "pageup": "Prior", "pgup": "Prior", "bildauf": "Prior", "pagedown": "Next", "pgdn": "Next", "bildab": "Next",
           "left": "Left", "right": "Right", "up": "Up", "down": "Down", "arrowleft": "Left", "arrowright": "Right",
           "arrowup": "Up", "arrowdown": "Down", "capslock": "Caps_Lock", "printscreen": "Print", "prtsc": "Print",
           "menu": "Menu", "apps": "Menu", "pause": "Pause", "numlock": "Num_Lock", "scrolllock": "Scroll_Lock",
           "plus": "plus", "minus": "minus", "comma": "comma", "period": "period",
           "playpause": "XF86AudioPlay", "play": "XF86AudioPlay", "nexttrack": "XF86AudioNext",
           "prevtrack": "XF86AudioPrev", "previoustrack": "XF86AudioPrev", "stop": "XF86AudioStop",
           "mediastop": "XF86AudioStop", "volumeup": "XF86AudioRaiseVolume", "volup": "XF86AudioRaiseVolume",
           "volumedown": "XF86AudioLowerVolume", "voldown": "XF86AudioLowerVolume", "mute": "XF86AudioMute",
           "volumemute": "XF86AudioMute", "numpadmultiply": "KP_Multiply", "numpadadd": "KP_Add",
           "numpadsubtract": "KP_Subtract", "numpaddecimal": "KP_Decimal", "numpaddivide": "KP_Divide",
           **{"f%d" % i: "F%d" % i for i in range(1, 25)}, **{"numpad%d" % i: "KP_%d" % i for i in range(10)},
           "=": "equal", "-": "minus", "[": "bracketleft", "]": "bracketright", ";": "semicolon", "'": "apostrophe",
           ",": "comma", ".": "period", "/": "slash", "\\": "backslash", "`": "grave", " ": "space"}
MODSYMS = {"ctrl": "Control_L", "control": "Control_L", "strg": "Control_L", "shift": "Shift_L", "umschalt": "Shift_L",
           "alt": "Alt_L", "option": "Alt_L", "altgr": "ISO_Level3_Shift", "win": "Super_L", "super": "Super_L",
           "meta": "Super_L", "cmd": "Super_L", "windows": "Super_L"}
_x = {}
_lock = threading.RLock()


def display():
    if "d" not in _x:
        from Xlib import display as xd
        _x["d"] = xd.Display() if os.environ.get("DISPLAY") else None
    return _x["d"]


def atom(name):
    d = display()
    return d.intern_atom(name) if d else 0


def prop(win_id, name):
    d = display()
    if not d:
        return None
    try:
        w = d.create_resource_object("window", win_id)
        p = w.get_full_property(atom(name), 0)
        return p.value if p else None
    except Exception:
        return None


def x_clients():
    d = display()
    if not d:
        return []
    v = prop(d.screen().root.id, "_NET_CLIENT_LIST_STACKING") or prop(d.screen().root.id, "_NET_CLIENT_LIST")
    return list(v or [])


def x_title(wid):
    v = prop(wid, "_NET_WM_NAME") or prop(wid, "WM_NAME")
    if v is None:
        return ""
    return v.decode("utf-8", "replace") if isinstance(v, bytes) else str(v)


def x_pid(wid):
    v = prop(wid, "_NET_WM_PID")
    return int(v[0]) if v is not None and len(v) else 0


def x_rect(wid):
    d = display()
    try:
        w = d.create_resource_object("window", wid)
        g = w.get_geometry()
        t = w.translate_coords(d.screen().root, 0, 0)
        x, y = -t.x, -t.y
        ext = prop(wid, "_NET_FRAME_EXTENTS")
        if ext is not None and len(ext) == 4:
            l, r, top, b = ext
            return x - l, y - top, x + g.width + r, y + g.height + b
        return x, y, x + g.width, y + g.height
    except Exception:
        return 0, 0, 0, 0


def x_state(wid):
    v = prop(wid, "_NET_WM_STATE")
    return set(v or [])


def proc_name(pid):
    try:
        with open("/proc/%d/comm" % pid) as f:
            return f.read().strip()
    except OSError:
        return ""


def proc_exe(pid):
    try:
        return os.readlink("/proc/%d/exe" % pid)
    except OSError:
        return ""


_electron = {}


def chromium_based(pid):
    if pid in _electron:
        return _electron[pid]
    exe = proc_exe(pid)
    base = os.path.dirname(exe)
    hit = proc_name(pid).lower() in CHROMIUM_APPS or any(os.path.exists(os.path.join(base, f)) for f in
                                                         ("resources/app.asar", "chrome_crashpad_handler", "chrome-sandbox"))
    _electron[pid] = hit
    return hit


def atspi():
    from lighting import uia_linux
    return uia_linux


def windows(owned=False):
    if WAYLAND or not display():
        return atspi().frames()
    fg = foreground()
    out = []
    for wid in reversed(x_clients()):
        st = x_state(wid)
        if atom("_NET_WM_STATE_SKIP_TASKBAR") in st and not owned:
            continue
        kind = prop(wid, "_NET_WM_WINDOW_TYPE") or []
        if kind and atom("_NET_WM_WINDOW_TYPE_NORMAL") not in kind and atom("_NET_WM_WINDOW_TYPE_DIALOG") not in kind:
            continue
        title = x_title(wid)
        if not title:
            continue
        l, t, r, b = x_rect(wid)
        if r - l < 40 or b - t < 30:
            continue
        pid = x_pid(wid)
        out.append({"hwnd": wid, "title": title, "cls": class_of(wid), "pid": pid, "exe": proc_name(pid),
                    "min": atom("_NET_WM_STATE_HIDDEN") in st, "fg": wid == fg})
    return out


def text_of(hwnd):
    if WAYLAND or not display():
        return atspi().frame_title(hwnd)
    return x_title(hwnd)


def class_of(hwnd):
    return class_of_pid(pid_of(hwnd))


def class_of_pid(pid):
    name = proc_name(pid).lower()
    if name in TERMINAL_APPS:
        return "terminal"
    if pid and chromium_based(pid):
        return CHROMIUM_CLASS
    return "x11" if not WAYLAND else "wayland"


def pid_of(hwnd):
    if WAYLAND or not display():
        return atspi().frame_pid(hwnd)
    return x_pid(hwnd)


def exe_of(pid):
    return proc_name(pid) if pid else ""


def elevated(pid):
    return False


def rect(hwnd):
    if WAYLAND or not display():
        return atspi().frame_rect(hwnd)
    return x_rect(hwnd)


def alive(hwnd):
    if not hwnd:
        return False
    if WAYLAND or not display():
        return atspi().frame_alive(hwnd)
    return hwnd in x_clients()


def visible(hwnd):
    return alive(hwnd) and not minimized(hwnd)


def minimized(hwnd):
    if WAYLAND or not display():
        return False
    return atom("_NET_WM_STATE_HIDDEN") in x_state(hwnd)


def client_message(wid, name, data):
    from Xlib import X
    from Xlib.protocol import event
    d = display()
    root = d.screen().root
    ev = event.ClientMessage(window=d.create_resource_object("window", wid), client_type=atom(name),
                             data=(32, (data + [0, 0, 0, 0, 0])[:5]))
    root.send_event(ev, event_mask=X.SubstructureRedirectMask | X.SubstructureNotifyMask)
    d.flush()


def close(hwnd):
    if WAYLAND or not display():
        atspi().frame_close(hwnd)
        return
    client_message(hwnd, "_NET_CLOSE_WINDOW", [0, 2])


def foreground():
    if WAYLAND or not display():
        return atspi().active_frame()
    d = display()
    v = prop(d.screen().root.id, "_NET_ACTIVE_WINDOW")
    return int(v[0]) if v is not None and len(v) and v[0] else 0


def set_foreground(hwnd):
    if WAYLAND or not display():
        return atspi().frame_focus(hwnd)
    client_message(hwnd, "_NET_ACTIVE_WINDOW", [2, 0, 0])
    deadline = time.time() + 0.6
    while time.time() < deadline:
        if foreground() == hwnd:
            return True
        time.sleep(0.03)
    return foreground() == hwnd


def restore_cycle(hwnd):
    if not WAYLAND and display():
        client_message(hwnd, "_NET_WM_STATE", [0, atom("_NET_WM_STATE_HIDDEN"), 0, 1])
        client_message(hwnd, "_NET_ACTIVE_WINDOW", [2, 0, 0])


def mutter_idle():
    try:
        out = subprocess.run(["gdbus", "call", "--session", "--dest", "org.gnome.Mutter.IdleMonitor", "--object-path",
                              "/org/gnome/Mutter/IdleMonitor/Core", "--method",
                              "org.gnome.Mutter.IdleMonitor.GetIdletime"], capture_output=True, text=True, timeout=2).stdout
        return int(out.strip("()\n ,").split()[-1].rstrip(","))
    except (OSError, ValueError, IndexError, subprocess.SubprocessError):
        return None


def idle_ms():
    if not WAYLAND and display():
        try:
            from Xlib.ext import screensaver
            return int(screensaver.query_info(display(), display().screen().root).idle)
        except Exception:
            pass
    v = mutter_idle()
    return v if v is not None else 10 ** 6


def cursor():
    d = display()
    if not d:
        return 0, 0
    p = d.screen().root.query_pointer()
    return p.root_x, p.root_y


def ydotool(*args):
    exe = shutil.which("ydotool")
    if not exe:
        raise ValueError("input on Wayland needs ydotool (sudo apt install ydotool, then start ydotoold)")
    subprocess.run([exe] + [str(a) for a in args], capture_output=True, timeout=10)


def xtest(kind, detail, x=None, y=None):
    from Xlib.ext import xtest as xt
    d = display()
    if x is not None:
        xt.fake_input(d, kind, detail, x=int(x), y=int(y), root=d.screen().root)
    else:
        xt.fake_input(d, kind, detail)
    d.sync()


def set_cursor(x, y):
    if WAYLAND or not display():
        ydotool("mousemove", "--absolute", "-x", int(x), "-y", int(y))
        return
    from Xlib import X
    xtest(X.MotionNotify, 0, x, y)


BUTTONS = {"left": 1, "middle": 2, "right": 3}
YDO_BUTTONS = {"left": "0xC0", "right": "0xC1", "middle": "0xC2"}


def click(button="left", count=1):
    if WAYLAND or not display():
        ydotool("click", "--repeat", count, YDO_BUTTONS[button])
        return
    from Xlib import X
    for _ in range(count):
        xtest(X.ButtonPress, BUTTONS[button])
        xtest(X.ButtonRelease, BUTTONS[button])
        time.sleep(0.02)


def mouse_down(button="left"):
    if WAYLAND or not display():
        ydotool("click", "0x4%d" % "lrm".index(button[0]))
        return
    from Xlib import X
    xtest(X.ButtonPress, BUTTONS[button])


def mouse_up(button="left"):
    if WAYLAND or not display():
        ydotool("click", "0x8%d" % "lrm".index(button[0]))
        return
    from Xlib import X
    xtest(X.ButtonRelease, BUTTONS[button])


def wheel(delta):
    notches = max(1, abs(int(round(delta / 120.0))))
    if WAYLAND or not display():
        ydotool("mousemove", "--wheel", "-x", 0, "-y", -notches if delta > 0 else notches)
        return
    from Xlib import X
    btn = 4 if delta > 0 else 5
    for _ in range(notches * 3):
        xtest(X.ButtonPress, btn)
        xtest(X.ButtonRelease, btn)


def combo_parts(combo):
    parts = [p for p in combo.replace(" ", "").split("+") if p] if combo != "+" else ["+"]
    if combo.endswith("++"):
        parts = parts + ["+"]
    return parts


def keysym_of(name):
    from Xlib import XK
    n = KEYSYMS.get(name.lower(), name if len(name) == 1 else name.capitalize())
    sym = XK.string_to_keysym(n)
    if not sym and len(name) == 1:
        sym = ord(name) if ord(name) < 0x100 else 0x01000000 + ord(name)
    if not sym:
        raise ValueError("unknown key '%s'" % name)
    return sym


def keycode_of(sym):
    d = display()
    code = d.keysym_to_keycode(sym)
    if code:
        return code, False
    spare = spare_code()
    d.change_keyboard_mapping(spare, [(sym, sym)])
    d.sync()
    return spare, True


def spare_code():
    d = display()
    if "spare" not in _x:
        first = d.display.info.min_keycode
        count = d.display.info.max_keycode - first + 1
        mapping = d.get_keyboard_mapping(first, count)
        free = [first + i for i, syms in enumerate(mapping) if not any(syms)]
        _x["spare"] = free[-1] if free else d.display.info.max_keycode
    return _x["spare"]


def press(combo, scancode=False):
    parts = combo_parts(combo)
    if WAYLAND or not display():
        ydotool("key", *ydo_keys(parts))
        return
    from Xlib import X
    *mods, last = parts
    for m in mods:
        if m.lower() not in MODSYMS:
            raise ValueError("unknown modifier '%s'" % m)
    codes = [keycode_of(keysym_of(MODSYMS[m.lower()]))[0] for m in mods]
    sym = keysym_of(last)
    code, temp = keycode_of(sym)
    shift = len(last) == 1 and last.isupper()
    with _lock:
        if shift:
            codes.append(keycode_of(keysym_of("Shift_L"))[0])
        for c in codes:
            xtest(X.KeyPress, c)
        xtest(X.KeyPress, code)
        xtest(X.KeyRelease, code)
        for c in reversed(codes):
            xtest(X.KeyRelease, c)


def ydo_keys(parts):
    codes = {"ctrl": 29, "shift": 42, "alt": 56, "super": 125, "win": 125, "cmd": 125, "meta": 125, "enter": 28,
             "tab": 15, "esc": 1, "escape": 1, "space": 57, "backspace": 14, "delete": 111, "home": 102, "end": 107,
             "pageup": 104, "pagedown": 109, "up": 103, "down": 108, "left": 105, "right": 106}
    letters = "qwertyuiop"
    row = {c: 16 + i for i, c in enumerate(letters)}
    row.update({c: 30 + i for i, c in enumerate("asdfghjkl")})
    row.update({c: 44 + i for i, c in enumerate("zxcvbnm")})
    row.update({str((i + 1) % 10): 2 + i for i in range(10)})
    seq = [codes.get(p.lower()) or row.get(p.lower()) for p in parts]
    if None in seq:
        raise ValueError("ydotool cannot send %s" % "+".join(parts))
    return ["%d:1" % c for c in seq] + ["%d:0" % c for c in reversed(seq)]


def type_unicode(text):
    if WAYLAND or not display():
        ydotool("type", text)
        return
    from Xlib import X
    for ch in text:
        if ch == "\n":
            press("enter")
            continue
        sym = keysym_of(ch)
        code, temp = keycode_of(sym)
        shift = display().keycode_to_keysym(code, 0) != sym and display().keycode_to_keysym(code, 1) == sym
        sc = keycode_of(keysym_of("Shift_L"))[0]
        if shift:
            xtest(X.KeyPress, sc)
        xtest(X.KeyPress, code)
        xtest(X.KeyRelease, code)
        if shift:
            xtest(X.KeyRelease, sc)


def clip_tool(write):
    if WAYLAND and shutil.which("wl-copy"):
        return ["wl-copy"] if write else ["wl-paste", "--no-newline"]
    if shutil.which("xclip"):
        return ["xclip", "-selection", "clipboard"] + ([] if write else ["-o"])
    if shutil.which("xsel"):
        return ["xsel", "--clipboard"] + (["--input"] if write else ["--output"])
    return None


def clip_get():
    tool = clip_tool(False)
    if not tool:
        return None
    try:
        return subprocess.run(tool, capture_output=True, timeout=3).stdout.decode("utf-8", "replace")
    except (OSError, subprocess.SubprocessError):
        return None


def clip_set(text):
    tool = clip_tool(True)
    if not tool:
        return False
    try:
        p = subprocess.Popen(tool, stdin=subprocess.PIPE, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                             start_new_session=True)
        p.stdin.write(text.encode("utf-8"))
        p.stdin.close()
        time.sleep(0.05)
        return True
    except OSError:
        return False


def post_click(hwnd):
    return False


def virtual_screen():
    d = display()
    if d:
        s = d.screen()
        return 0, 0, s.width_in_pixels, s.height_in_pixels
    return 0, 0, 1920, 1080


def print_window(hwnd):
    if WAYLAND or not display():
        return None
    try:
        from PIL import Image
        from Xlib import X
        d = display()
        w = d.create_resource_object("window", hwnd)
        g = w.get_geometry()
        raw = w.get_image(0, 0, g.width, g.height, X.ZPixmap, 0xFFFFFFFF)
        img = Image.frombytes("RGB", (g.width, g.height), raw.data, "raw", "BGRX")
        return img if img.getbbox() else None
    except Exception:
        return None


def portal_shot():
    exe = shutil.which("gnome-screenshot") or shutil.which("grim")
    if not exe:
        return None
    import tempfile
    from PIL import Image
    path = os.path.join(tempfile.gettempdir(), "lighting-shot-%d.png" % os.getpid())
    args = [exe, "-f", path] if exe.endswith("gnome-screenshot") else [exe, path]
    subprocess.run(args, capture_output=True, timeout=15)
    try:
        img = Image.open(path)
        img.load()
        return img
    finally:
        try:
            os.unlink(path)
        except OSError:
            pass


def screen_grab(box):
    from PIL import ImageGrab
    if not WAYLAND and display():
        return ImageGrab.grab(bbox=box, xdisplay=os.environ.get("DISPLAY"))
    img = portal_shot()
    if img is None:
        raise ValueError("screenshots on Wayland need gnome-screenshot or grim")
    return img.crop(box)


def video_input(x0, y0, size, fps):
    if WAYLAND:
        raise ValueError("screen video on Wayland is not supported yet, use an X11 session")
    return ["-f", "x11grab", "-framerate", str(fps), "-video_size", size, "-i",
            "%s+%d,%d" % (os.environ.get("DISPLAY", ":0"), x0, y0)]


def desktop_dirs():
    home = os.path.expanduser("~")
    data = os.environ.get("XDG_DATA_DIRS", "/usr/local/share:/usr/share").split(":")
    return [os.path.join(home, ".local", "share", "applications")] + [os.path.join(d, "applications") for d in data] + \
        ["/var/lib/snapd/desktop/applications", "/var/lib/flatpak/exports/share/applications",
         os.path.join(home, ".local", "share", "flatpak", "exports", "share", "applications")]


def app_list():
    out, seen = [], set()
    for d in desktop_dirs():
        try:
            names = sorted(os.listdir(d))
        except OSError:
            continue
        for n in names:
            if not n.endswith(".desktop") or n in seen:
                continue
            seen.add(n)
            path = os.path.join(d, n)
            entry = {}
            try:
                with open(path, encoding="utf-8", errors="replace") as f:
                    section = ""
                    for line in f:
                        line = line.strip()
                        if line.startswith("["):
                            section = line
                        elif section == "[Desktop Entry]" and "=" in line:
                            k, v = line.split("=", 1)
                            entry.setdefault(k, v)
            except OSError:
                continue
            if entry.get("NoDisplay") == "true" or entry.get("Type", "Application") != "Application":
                continue
            name = entry.get("Name") or n[:-8]
            out.append((name, path))
    return out


def run_name(name):
    return shutil.which(name)


def open_target(target):
    exe = shutil.which("xdg-open") or "xdg-open"
    if shutil.which(target) and not os.path.exists(target):
        subprocess.Popen([shutil.which(target)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        return
    subprocess.Popen([exe, target], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)


def start_app(app):
    name = os.path.basename(app[1])[:-8]
    for cmd in (["gtk-launch", name], ["gio", "launch", app[1]]):
        if shutil.which(cmd[0]):
            subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
            return
    raise ValueError("cannot start %s (needs gtk-launch or gio)" % app[0])


def file_dialog(browser_hwnd, before):
    pid = pid_of(browser_hwnd)
    for w in windows(owned=True):
        if w["hwnd"] not in before and w["pid"] == pid and w["hwnd"] != browser_hwnd:
            return w["hwnd"]
    return None


def choose_folder(dlg, path):
    set_foreground(dlg)
    time.sleep(0.3)
    press("ctrl+l")
    time.sleep(0.3)
    if clip_set(path):
        press("ctrl+a")
        press("ctrl+v")
    else:
        type_unicode(path)
    time.sleep(0.4)
    press("enter")
    time.sleep(0.8)
    if alive(dlg):
        from lighting import uia_linux
        uia_linux.press_named(dlg, ("select", "auswählen", "öffnen", "open"))
    return True


def window_at(x, y):
    for w in windows():
        l, t, r, b = rect(w["hwnd"])
        if l <= x < r and t <= y < b and not w["min"]:
            return w["hwnd"]
    return 0


def hotkey_loop(daemon):
    if WAYLAND or not display():
        return
    from Xlib import X, display as xd
    d = xd.Display()
    root = d.screen().root
    code = d.keysym_to_keycode(keysym_of("End"))
    mods = X.ControlMask | X.Mod1Mask
    for extra in (0, X.LockMask, X.Mod2Mask, X.LockMask | X.Mod2Mask):
        root.grab_key(code, mods | extra, True, X.GrabModeAsync, X.GrabModeAsync)
    d.sync()
    daemon.hotkey = True
    while True:
        ev = d.next_event()
        if ev.type == X.KeyPress and ev.detail == code:
            daemon.abort_all()


def listen_input(q, stop, ready, injected=False):
    if WAYLAND or not display():
        ready.set()
        return
    from Xlib import X, XK, display as xd
    from Xlib.ext import record
    from Xlib.protocol import rq
    ctl, data = xd.Display(), xd.Display()
    ctx = ctl.record_create_context(0, [record.AllClients], [{
        "core_requests": (0, 0), "core_replies": (0, 0), "ext_requests": (0, 0, 0, 0), "ext_replies": (0, 0, 0, 0),
        "delivered_events": (0, 0), "device_events": (X.KeyPress, X.ButtonPress), "errors": (0, 0),
        "client_started": False, "client_died": False}])
    names = {}
    for ours, xname in reversed(list(KEYSYMS.items())):
        sym = XK.string_to_keysym(xname)
        if sym:
            names[sym] = ours

    def cb(reply):
        if reply.category != record.FromServer or reply.client_swapped or not len(reply.data):
            return
        buf = reply.data
        while len(buf):
            ev, buf = rq.EventField(None).parse_binary_value(buf, ctl.display, None, None)
            t = time.time()
            if ev.type == X.ButtonPress and ev.detail in (1, 3):
                q.put(("m", ev.root_x, ev.root_y, ev.detail == 3, t))
            elif ev.type == X.KeyPress:
                shift = bool(ev.state & X.ShiftMask)
                sym = ctl.keycode_to_keysym(ev.detail, 1 if shift else 0)
                s = XK.keysym_to_string(sym) or ""
                mods = [n for m, n in ((X.ControlMask, "ctrl"), (X.Mod1Mask, "alt"), (X.ShiftMask, "shift"),
                                       (X.Mod4Mask, "win")) if ev.state & m]
                base = ctl.keycode_to_keysym(ev.detail, 0)
                low = (XK.keysym_to_string(base) or "").lower()
                q.put(("k", names.get(base) or (low if len(low) == 1 else ""), s if len(s) == 1 else "", mods, t))

    ready.set()
    th = threading.Thread(target=lambda: data.record_enable_context(ctx, cb), daemon=True)
    th.start()
    stop.wait()
    ctl.record_disable_context(ctx)
    ctl.flush()
    th.join(2)
    ctl.record_free_context(ctx)


def terminal_app():
    return next((t for t in ("gnome-terminal", "ptyxis", "konsole", "x-terminal-emulator", "xterm") if shutil.which(t)), "xterm")


def access(prompt=False):
    out = {"display": bool(display()) or WAYLAND}
    try:
        from lighting import uia_linux
        uia_linux.A()
        out["at-spi"] = True
    except Exception:
        out["at-spi"] = False
    if WAYLAND:
        out["ydotool"] = bool(shutil.which("ydotool"))
    out["clipboard tool"] = clip_tool(True) is not None
    return out

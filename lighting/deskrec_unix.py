import os
import queue
import threading
import time

from lighting import uia
from lighting import win

SKIP_EXES = {"brave browser", "google chrome", "microsoft edge", "chromium", "opera", "vivaldi", "arc", "brave", "chrome",
             "msedge", "chromium-browser", "firefox", "terminal", "iterm2", "ghostty", "wezterm-gui", "alacritty", "kitty",
             "warp", "code", "cursor", "windsurf", "claude", "gnome-terminal-server", "konsole", "xterm", "tilix",
             "ptyxis", "gnome-shell", "dock", "window server", "systemuiserver"}
ACTIONABLE = {"button", "checkbox", "combobox", "link", "listitem", "menuitem", "radio", "tab", "treeitem", "dataitem",
              "edit", "splitbutton"}
SPECIAL = {"enter", "escape", "esc", "delete", "home", "end", "pageup", "pagedown", "up", "down", "left", "right",
           "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12", "space", "insert"}


class DeskRecorder:
    def __init__(self, minutes=15, injected=False):
        self.minutes = minutes
        self.injected = injected
        self.q = queue.Queue()
        self.steps = []
        self.stop_flag = threading.Event()
        self.ready = threading.Event()
        self.last_root = None
        self.buf, self.buf_root, self.buf_target, self.buf_t = "", None, None, 0.0
        self.pids0 = set()
        self.started = {}
        self.me = os.getpid()
        self.timer = None
        self.busy = None
        self.skipped = []

    def start(self):
        self.pids0 = {w["pid"] for w in win.windows()}
        self.hook_thread = threading.Thread(target=self._hooks, daemon=True)
        self.work_thread = threading.Thread(target=self._work, daemon=True)
        self.hook_thread.start()
        self.work_thread.start()
        self.ready.wait(3)
        self.timer = threading.Timer(self.minutes * 60, self.stop_flag.set)
        self.timer.daemon = True
        self.timer.start()
        return self

    def stop(self):
        if self.timer:
            self.timer.cancel()
        self.stop_flag.set()
        self.hook_thread.join(3)
        self.q.put(None)
        self.work_thread.join(5)
        note = None
        if self.work_thread.is_alive():
            note = "the app recorder was still reading %s after 5 s, it is not recorded" % (self.busy or "an input")
        elif self.skipped and all(s["cmd"] == "#" for s in self.steps):
            note = "%d click%s went to %s, which the app recorder leaves out" % (
                len(self.skipped), "" if len(self.skipped) == 1 else "s", ", ".join(sorted(set(self.skipped))))
        if note:
            self.steps.append({"t": time.time(), "cmd": "#", "args": [note], "flags": {}})
        return self.steps

    def _hooks(self):
        try:
            win.listen_input(self.q, self.stop_flag, self.ready, self.injected)
        finally:
            self.ready.set()

    def _work(self):
        while True:
            item = self.q.get()
            if item is None:
                self._flush()
                return
            self.busy = "a click" if item[0] == "m" else "a key"
            try:
                if item[0] == "m":
                    self._flush()
                    self._click(*item[1:])
                else:
                    self._key(*item[1:])
            except Exception:
                pass
            self.busy = None

    def _emit(self, cmd, args, t, **flags):
        flags["on"] = "app"
        self.steps.append({"t": t, "cmd": cmd, "args": args, "flags": flags})

    def _skip(self, root):
        if not root:
            return True
        pid = win.pid_of(root)
        return pid == self.me or (win.exe_of(pid) or "").lower() in SKIP_EXES

    def _context(self, root, t):
        if root == self.last_root:
            return
        self.last_root = root
        pid = win.pid_of(root)
        key = win.exe_of(pid) or win.text_of(root)
        if pid not in self.pids0 and not self.started.get(key):
            self.started[key] = True
            self._emit("launch", [key], t)
        else:
            self._emit("focus", ["app:" + key], t)

    def _click(self, x, y, right, t):
        self.busy = "a click (the window under the mouse)"
        root = win.window_at(x, y)
        if self._skip(root):
            self.skipped.append((win.exe_of(win.pid_of(root)) or "an unknown window") if root else "the desktop")
            return
        self.busy = "a click (the control under the mouse)"
        cur, pick, text_name = uia.element_at(x, y), None, ""
        self.busy = "a click (the control's name)"
        for _ in range(6):
            if cur is None:
                break
            role, name, _ = uia.describe(cur)
            if role == "text" and name and not text_name:
                text_name = name
            if role in ACTIONABLE and (name or role == "edit"):
                pick = (role, name)
                break
            cur = uia.parent_of(cur)
        if pick and pick[0] == "edit":
            return
        name = pick[1] if pick else text_name
        self._context(root, t)
        if not name:
            self.steps.append({"t": t, "cmd": "#", "args": ["click at %d,%d had no name, not recorded" % (x, y)], "flags": {}})
            return
        self._emit("click", [name[:80]], t, **({"right": True} if right else {}))

    def _focused(self):
        el = uia.focused()
        if el is None:
            return ("", False)
        role, name, extra = uia.describe(el)
        return (name[:60], "=***" in extra)

    def _flush(self):
        if not self.buf:
            return
        name, secret = self.buf_target or ("", False)
        self._context(self.buf_root, self.buf_t)
        self._emit("type", [name or "focused", "@secret" if secret else self.buf], self.buf_t)
        self.buf, self.buf_root, self.buf_target = "", None, None

    def _key(self, name, ch, mods, t):
        root = win.foreground()
        if self._skip(root):
            return
        cmd = [m for m in mods if m in ("ctrl", "alt", "cmd", "win")]
        if ch and ch.isprintable() and not cmd:
            if self.buf_root != root:
                self._flush()
                self.buf_root, self.buf_target, self.buf_t = root, self._focused(), t
            self.buf += ch
            return
        if name == "backspace" and self.buf:
            self.buf = self.buf[:-1]
            return
        if name == "tab" and not cmd:
            self._flush()
            return
        if not name or (name not in SPECIAL and not cmd):
            return
        self._flush()
        self._context(root, t)
        self._emit("press", ["+".join(mods + [name])], t)

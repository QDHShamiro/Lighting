import ctypes
import os
import queue
import re
import threading
import time
from ctypes import wintypes as W

from lighting import win

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
HOOKPROC = ctypes.WINFUNCTYPE(ctypes.c_ssize_t, ctypes.c_int, W.WPARAM, W.LPARAM)
user32.SetWindowsHookExW.argtypes = [ctypes.c_int, HOOKPROC, W.HINSTANCE, W.DWORD]
user32.SetWindowsHookExW.restype = W.HHOOK
user32.CallNextHookEx.argtypes = [W.HHOOK, ctypes.c_int, W.WPARAM, W.LPARAM]
user32.CallNextHookEx.restype = ctypes.c_ssize_t
user32.UnhookWindowsHookEx.argtypes = [W.HHOOK]
user32.GetMessageW.argtypes = [ctypes.POINTER(W.MSG), W.HWND, W.UINT, W.UINT]
user32.PostThreadMessageW.argtypes = [W.DWORD, W.UINT, W.WPARAM, W.LPARAM]
user32.WindowFromPoint.argtypes = [W.POINT]
user32.WindowFromPoint.restype = W.HWND
user32.GetAncestor.argtypes = [W.HWND, W.UINT]
user32.GetAncestor.restype = W.HWND
user32.GetForegroundWindow.restype = W.HWND
user32.GetWindowThreadProcessId.argtypes = [W.HWND, ctypes.POINTER(W.DWORD)]
user32.GetKeyboardLayout.argtypes = [W.DWORD]
user32.GetKeyboardLayout.restype = W.HKL
user32.ToUnicodeEx.argtypes = [W.UINT, W.UINT, ctypes.POINTER(ctypes.c_ubyte), W.LPWSTR, ctypes.c_int, W.UINT, W.HKL]
user32.GetAsyncKeyState.argtypes = [ctypes.c_int]
user32.GetAsyncKeyState.restype = ctypes.c_short
user32.GetKeyState.argtypes = [ctypes.c_int]
user32.GetKeyState.restype = ctypes.c_short
kernel32.GetModuleHandleW.argtypes = [W.LPCWSTR]
kernel32.GetModuleHandleW.restype = W.HMODULE


class MSLL(ctypes.Structure):
    _fields_ = [("x", W.LONG), ("y", W.LONG), ("data", W.DWORD), ("flags", W.DWORD), ("time", W.DWORD),
                ("extra", ctypes.c_size_t)]


class KBLL(ctypes.Structure):
    _fields_ = [("vk", W.DWORD), ("scan", W.DWORD), ("flags", W.DWORD), ("time", W.DWORD),
                ("extra", ctypes.c_size_t)]


SKIP_EXES = {"brave.exe", "chrome.exe", "msedge.exe", "firefox.exe", "opera.exe", "vivaldi.exe", "chromium.exe",
             "windowsterminal.exe", "openconsole.exe", "conhost.exe", "cmd.exe", "powershell.exe", "pwsh.exe",
             "mintty.exe", "wezterm-gui.exe", "alacritty.exe", "code.exe", "cursor.exe", "windsurf.exe",
             "claude.exe", "searchhost.exe", "startmenuexperiencehost.exe", "shellexperiencehost.exe"}
SKIP_CLASSES = {"Shell_TrayWnd", "Shell_SecondaryTrayWnd", "Progman", "WorkerW", "NotifyIconOverflowWindow"}
ACTIONABLE = {"button", "checkbox", "combobox", "link", "listitem", "menuitem", "radio", "tab", "treeitem",
              "dataitem", "splitbutton", "slider", "header", "edit"}
MOD_VKS = {0x10, 0x11, 0x12, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0x14, 0x90, 0x91}
VK_NAME = {0x0D: "enter", 0x1B: "escape", 0x20: "space", 0x2E: "delete", 0x2D: "insert", 0x24: "home", 0x23: "end",
           0x21: "pageup", 0x22: "pagedown", 0x25: "left", 0x26: "up", 0x27: "right", 0x28: "down", 0x08: "backspace",
           0x09: "tab"}


def vk_name(vk):
    if vk in VK_NAME:
        return VK_NAME[vk]
    if 0x70 <= vk <= 0x87:
        return "f%d" % (vk - 0x6F)
    if 0x41 <= vk <= 0x5A or 0x30 <= vk <= 0x39:
        return chr(vk).lower()
    return None


def down(vk):
    return bool(user32.GetAsyncKeyState(vk) & 0x8000)


def app_key(exe, title):
    stem = re.sub(r"\.exe$", "", exe or "", flags=re.I)
    if stem.lower() == "applicationframehost":
        return title or stem
    return stem


class DeskRecorder:
    def __init__(self, minutes=15, injected=False):
        self.minutes = minutes
        self.injected = injected
        self.q = queue.Queue()
        self.steps = []
        self.ready = threading.Event()
        self.tid = None
        self.last_root = None
        self.buf, self.buf_root, self.buf_target, self.buf_t = "", None, None, 0.0
        self.pids0 = set()
        self.started = {}
        self.me = os.getpid()
        self.timer = None

    def start(self):
        self.pids0 = {w["pid"] for w in win.windows()}
        self.hook_thread = threading.Thread(target=self._hooks, daemon=True)
        self.work_thread = threading.Thread(target=self._work, daemon=True)
        self.hook_thread.start()
        self.work_thread.start()
        self.ready.wait(3)
        self.timer = threading.Timer(self.minutes * 60, self._unhook)
        self.timer.daemon = True
        self.timer.start()
        return self

    def stop(self):
        if self.timer:
            self.timer.cancel()
        self._unhook()
        self.hook_thread.join(3)
        self.q.put(None)
        self.work_thread.join(5)
        return self.steps

    def _unhook(self):
        if self.tid:
            user32.PostThreadMessageW(self.tid, 0x0012, 0, 0)

    def _hooks(self):
        self.tid = kernel32.GetCurrentThreadId()
        self._mproc = HOOKPROC(self._on_mouse)
        self._kproc = HOOKPROC(self._on_key)
        mod = kernel32.GetModuleHandleW(None)
        hm = user32.SetWindowsHookExW(14, self._mproc, mod, 0)
        hk = user32.SetWindowsHookExW(13, self._kproc, mod, 0)
        self.ready.set()
        msg = W.MSG()
        while user32.GetMessageW(ctypes.byref(msg), None, 0, 0) > 0:
            pass
        if hm:
            user32.UnhookWindowsHookEx(hm)
        if hk:
            user32.UnhookWindowsHookEx(hk)

    def _on_mouse(self, code, wp, lp):
        if code == 0 and wp in (0x0201, 0x0204):
            info = MSLL.from_address(lp)
            if self.injected or not info.flags & 0x01:
                self.q.put(("m", info.x, info.y, wp == 0x0204, time.time()))
        return user32.CallNextHookEx(None, code, wp, lp)

    def _on_key(self, code, wp, lp):
        if code == 0 and wp in (0x0100, 0x0104):
            info = KBLL.from_address(lp)
            if self.injected or not info.flags & 0x10:
                mods = (down(0x10), down(0x11), down(0x12), down(0x5B) or down(0x5C), bool(user32.GetKeyState(0x14) & 1))
                fg = user32.GetForegroundWindow()
                hkl = user32.GetKeyboardLayout(user32.GetWindowThreadProcessId(fg, None))
                self.q.put(("k", info.vk, info.scan, mods, fg, hkl, time.time()))
        return user32.CallNextHookEx(None, code, wp, lp)

    def _work(self):
        import comtypes
        import comtypes.client
        try:
            comtypes.CoInitializeEx(comtypes.COINIT_APARTMENTTHREADED)
        except OSError:
            pass
        comtypes.client.GetModule("UIAutomationCore.dll")
        from comtypes.gen import UIAutomationClient as U
        self.U = U
        self.uia = comtypes.client.CreateObject(U.CUIAutomation8, interface=U.IUIAutomation)
        while True:
            ev = self.q.get()
            if ev is None:
                break
            try:
                if ev[0] == "m":
                    self._flush()
                    self._click(*ev[1:])
                else:
                    self._key(*ev[1:])
            except Exception:
                pass
        self._flush()

    def _emit(self, cmd, args, t, **flags):
        flags["on"] = "app"
        self.steps.append({"t": t, "cmd": cmd, "args": args, "flags": flags})

    def _skip(self, root):
        if not root:
            return True
        pid = win.pid_of(root)
        if pid == self.me:
            return True
        if win.class_of(root) in SKIP_CLASSES:
            return True
        return (win.exe_of(pid) or "").lower() in SKIP_EXES

    def _context(self, root, t):
        if root == self.last_root:
            return
        self.last_root = root
        pid = win.pid_of(root)
        exe, title = win.exe_of(pid), win.text_of(root)
        key = app_key(exe, title)
        if pid not in self.pids0 and not self.started.get(key):
            self.started[key] = True
            self._emit("launch", [key], t)
        else:
            self._emit("focus", ["app:" + key], t)

    def _role(self, el):
        from lighting.uia import ROLES
        try:
            return ROLES.get(el.CurrentControlType, "text" if el.CurrentControlType == 50020 else "other")
        except Exception:
            return "other"

    def _click(self, x, y, right, t):
        root = user32.GetAncestor(user32.WindowFromPoint(W.POINT(x, y)), 2)
        if self._skip(root):
            return
        el = self.uia.ElementFromPoint(self.U.tagPOINT(x, y))
        walker = self.uia.ControlViewWalker
        pick, text_name = None, ""
        cur = el
        for _ in range(5):
            if cur is None:
                break
            role, name = self._role(cur), (cur.CurrentName or "").strip()
            if role == "text" and name and not text_name:
                text_name = name
            if role in ACTIONABLE and (name or role == "edit"):
                pick = (role, name)
                break
            try:
                cur = walker.GetParentElement(cur)
            except Exception:
                break
        if pick and pick[0] == "edit":
            return
        name = pick[1] if pick else text_name
        self._context(root, t)
        if not name:
            self.steps.append({"t": t, "cmd": "#", "args": ["click at %d,%d had no name, not recorded" % (x, y)], "flags": {}})
            return
        self._emit("click", [name[:80]], t, **({"right": True} if right else {}))

    def _focused(self, root):
        try:
            el = self.uia.GetFocusedElement()
            if el is None or el.CurrentProcessId != win.pid_of(root):
                return ("", False)
            return ((el.CurrentName or "").strip()[:60], bool(el.CurrentIsPassword))
        except Exception:
            return ("", False)

    def _flush(self):
        if not self.buf:
            return
        name, secret = self.buf_target or ("", False)
        self._context(self.buf_root, self.buf_t)
        self._emit("type", [name or "focused", "@secret" if secret else self.buf], self.buf_t)
        self.buf, self.buf_root, self.buf_target = "", None, None

    def _key(self, vk, scan, mods, fg, hkl, t):
        shift, ctrl, alt, winkey, caps = mods
        root = user32.GetAncestor(fg, 2) if fg else None
        if self._skip(root) or vk in MOD_VKS:
            return
        ch = ""
        if not winkey:
            state = (ctypes.c_ubyte * 256)()
            if shift:
                state[0x10] = 0x80
            if ctrl:
                state[0x11] = 0x80
            if alt:
                state[0x12] = 0x80
            if caps:
                state[0x14] = 0x01
            buf = ctypes.create_unicode_buffer(8)
            n = user32.ToUnicodeEx(vk, scan, state, buf, 8, 0x4, hkl)
            ch = buf.value[:n] if n > 0 else ""
        if ch and ch.isprintable() and (not (ctrl or alt) or (ctrl and alt)):
            if self.buf_root != root:
                self._flush()
                self.buf_root, self.buf_target, self.buf_t = root, self._focused(root), t
            self.buf += ch
            return
        if vk == 0x08 and self.buf:
            self.buf = self.buf[:-1]
            return
        name = vk_name(vk)
        if not name:
            return
        if vk == 0x09 and not (ctrl or alt):
            self._flush()
            return
        mods_on = [m for m, on in (("ctrl", ctrl), ("alt", alt), ("shift", shift), ("win", winkey)) if on]
        self._flush()
        self._context(root, t)
        self._emit("press", ["+".join(mods_on + [name])], t)

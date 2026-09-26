import queue
import threading
import time

from lighting import defaults as D
from lighting import win
from lighting.common import Fail

_q = queue.Queue()
_started = []


def _loop():
    import ctypes
    import tkinter as tk
    root = tk.Tk()
    root.withdraw()
    root.overrideredirect(True)
    root.attributes("-topmost", True)
    key = "#010203"
    root.configure(bg=key)
    root.attributes("-transparentcolor", key)
    canvas = tk.Canvas(root, width=26, height=32, bg=key, highlightthickness=0)
    canvas.pack()
    canvas.create_polygon(2, 2, 2, 24, 8, 18, 12, 28, 16, 26, 12, 17, 21, 17, fill=D.POINTER_COLOR, outline="white", width=2)
    root.update_idletasks()
    hwnd = ctypes.windll.user32.GetParent(root.winfo_id()) or root.winfo_id()
    style = ctypes.windll.user32.GetWindowLongW(hwnd, -20)
    ctypes.windll.user32.SetWindowLongW(hwnd, -20, style | 0x20 | 0x80000 | 0x80 | 0x08000000 | 0x8)
    state = {"hide": None, "badge": None}

    def badge(on):
        if on and state["badge"] is None:
            top = tk.Toplevel(root)
            top.overrideredirect(True)
            top.attributes("-topmost", True)
            top.attributes("-alpha", 0.92)
            tk.Label(top, text="● REC Lighting", fg="white", bg="#d00000", font=("Segoe UI", 10, "bold"),
                     padx=10, pady=3).pack()
            top.update_idletasks()
            top.geometry("+%d+%d" % (top.winfo_screenwidth() - top.winfo_reqwidth() - 18, 14))
            h = ctypes.windll.user32.GetParent(top.winfo_id()) or top.winfo_id()
            ex = ctypes.windll.user32.GetWindowLongW(h, -20)
            ctypes.windll.user32.SetWindowLongW(h, -20, ex | 0x20 | 0x80 | 0x08000000 | 0x8)
            state["badge"] = top
        elif not on and state["badge"] is not None:
            state["badge"].destroy()
            state["badge"] = None

    def poll():
        try:
            while True:
                item = _q.get_nowait()
                if isinstance(item, tuple) and item and item[0] == "rec":
                    badge(item[1])
                    continue
                if item is None:
                    if state["hide"]:
                        root.after_cancel(state["hide"])
                        state["hide"] = None
                    root.withdraw()
                    continue
                x, y = item
                root.geometry("+%d+%d" % (x - 2, y - 2))
                root.deiconify()
                root.lift()
                root.attributes("-topmost", True)
                if state["hide"]:
                    root.after_cancel(state["hide"])
                state["hide"] = root.after(D.POINTER_MS, root.withdraw)
        except queue.Empty:
            pass
        root.after(30, poll)

    poll()
    root.mainloop()


def show(x, y):
    if not D.config().get("pointer", True):
        return
    if not _started:
        _started.append(True)
        threading.Thread(target=_loop, daemon=True).start()
    _q.put((int(x), int(y)))


def recording(on):
    if not _started:
        _started.append(True)
        threading.Thread(target=_loop, daemon=True).start()
    _q.put(("rec", bool(on)))


def hide():
    if _started:
        _q.put(None)
        time.sleep(0.06)


def wait_idle():
    deadline = time.time() + D.BLITZ_WAIT_MS / 1000
    while win.idle_ms() < D.IDLE_MS:
        if time.time() > deadline:
            return False
        time.sleep(0.03)
    return True


def front(hwnd):
    if win.set_foreground(hwnd):
        return True
    try:
        from lighting import uia
        uia.api()[0].ElementFromHandle(hwnd).SetFocus()
        time.sleep(0.06)
        if win.foreground() == hwnd:
            return True
    except Exception:
        pass
    win.user32.ShowWindow(hwnd, 6)
    time.sleep(0.05)
    win.user32.ShowWindow(hwnd, 9)
    time.sleep(0.12)
    return win.foreground() == hwnd or win.set_foreground(hwnd)


def blitz_click(x, y, button="left", count=1, hwnd=None):
    show(x, y)
    if not wait_idle():
        raise Fail("you are using mouse or keyboard right now (waited %ds)" % (D.BLITZ_WAIT_MS // 1000), "retry in a moment")
    prev_fg = win.foreground()
    if hwnd:
        front(hwnd)
    saved = win.cursor()
    try:
        win.set_cursor(x, y)
        time.sleep(0.01)
        win.click(button, count)
        time.sleep(0.01)
    finally:
        win.set_cursor(*saved)
    return prev_fg


def blitz_drag(a, b, hwnd=None):
    show(*a)
    if not wait_idle():
        raise Fail("you are using mouse or keyboard right now", "retry in a moment")
    if hwnd:
        front(hwnd)
    saved = win.cursor()
    try:
        win.set_cursor(*a)
        win.mouse_down()
        steps = 12
        for i in range(1, steps + 1):
            win.set_cursor(a[0] + (b[0] - a[0]) * i // steps, a[1] + (b[1] - a[1]) * i // steps)
            time.sleep(0.012)
        win.mouse_up()
    finally:
        win.set_cursor(*saved)
    show(*b)


def with_focus(hwnd, fn, stay=False):
    if not wait_idle():
        raise Fail("you are using mouse or keyboard right now", "retry in a moment")
    prev = win.foreground()
    if not front(hwnd):
        raise Fail("Windows refused to bring the window to the front, keys were not sent",
                   "click the window once, or use click/type on d-refs (background)")
    try:
        return fn()
    finally:
        if not stay and prev and prev != hwnd and win.alive(prev):
            time.sleep(0.05)
            win.set_foreground(prev)

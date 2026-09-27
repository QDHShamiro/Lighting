import ctypes
import ctypes.wintypes as W

user32 = ctypes.WinDLL("user32", use_last_error=True)
kernel32 = ctypes.WinDLL("kernel32")
gdi32 = ctypes.WinDLL("gdi32")

LRESULT = ctypes.c_ssize_t
WNDPROC = ctypes.WINFUNCTYPE(LRESULT, W.HWND, W.UINT, W.WPARAM, W.LPARAM)
user32.DefWindowProcW.argtypes = [W.HWND, W.UINT, W.WPARAM, W.LPARAM]
user32.DefWindowProcW.restype = LRESULT
user32.CreateWindowExW.argtypes = [W.DWORD, W.LPCWSTR, W.LPCWSTR, W.DWORD, ctypes.c_int, ctypes.c_int, ctypes.c_int,
                                   ctypes.c_int, W.HWND, W.HMENU, W.HINSTANCE, W.LPVOID]
user32.CreateWindowExW.restype = W.HWND
user32.SetWindowTextW.argtypes = [W.HWND, W.LPCWSTR]
user32.SendMessageW.argtypes = [W.HWND, W.UINT, W.WPARAM, W.LPARAM]
user32.SendMessageW.restype = LRESULT
kernel32.GetModuleHandleW.restype = W.HMODULE
user32.LoadCursorW.argtypes = [W.HINSTANCE, W.LPVOID]
user32.LoadCursorW.restype = W.HANDLE
gdi32.GetStockObject.restype = ctypes.c_void_p


class WNDCLASSW(ctypes.Structure):
    _fields_ = [("style", W.UINT), ("lpfnWndProc", WNDPROC), ("cbClsExtra", ctypes.c_int), ("cbWndExtra", ctypes.c_int),
                ("hInstance", W.HINSTANCE), ("hIcon", W.HICON), ("hCursor", W.HANDLE), ("hbrBackground", W.HBRUSH),
                ("lpszMenuName", W.LPCWSTR), ("lpszClassName", W.LPCWSTR)]


state = {"clicks": 0}


def main():
    inst = kernel32.GetModuleHandleW(None)

    def proc(hwnd, msg, wp, lp):
        if msg == 0x0111 and (wp & 0xFFFF) == 101:
            state["clicks"] += 1
            buf = ctypes.create_unicode_buffer(256)
            user32.GetWindowTextW(state["edit"], buf, 256)
            user32.SetWindowTextW(state["status"], "Status: clicked %d with %s" % (state["clicks"], buf.value or "nothing"))
            return 0
        if msg == 0x0002:
            user32.PostQuitMessage(0)
            return 0
        return user32.DefWindowProcW(hwnd, msg, wp, lp)

    wndproc = WNDPROC(proc)
    wc = WNDCLASSW(0, wndproc, 0, 0, inst, None, user32.LoadCursorW(None, 32512), 16, None, "LightingTestApp")
    user32.RegisterClassW(ctypes.byref(wc))
    hwnd = user32.CreateWindowExW(0, "LightingTestApp", "Lighting Test App", 0x10CF0000, 200, 200, 460, 260,
                                  None, None, inst, None)
    font = gdi32.GetStockObject(17)
    child = 0x50000000

    def make(cls, text, style, x, y, w, h, cid):
        c = user32.CreateWindowExW(0x200 if cls == "EDIT" else 0, cls, text, child | style, x, y, w, h, hwnd, cid, inst, None)
        user32.SendMessageW(c, 0x0030, font, 1)
        return c

    make("STATIC", "Name:", 0, 20, 24, 60, 22, 0)
    state["edit"] = make("EDIT", "", 0x00010080, 90, 20, 320, 26, 100)
    make("BUTTON", "Go", 0x00010000, 90, 64, 100, 32, 101)
    make("BUTTON", "Remember me", 0x00010003, 210, 64, 200, 32, 102)
    state["status"] = make("STATIC", "Status: ready", 0, 20, 120, 400, 24, 103)
    user32.ShowWindow(hwnd, 5)
    msg = W.MSG()
    while user32.GetMessageW(ctypes.byref(msg), None, 0, 0) > 0:
        if not user32.IsDialogMessageW(hwnd, ctypes.byref(msg)):
            user32.TranslateMessage(ctypes.byref(msg))
            user32.DispatchMessageW(ctypes.byref(msg))


def chat():
    inst = kernel32.GetModuleHandleW(None)
    box = {}

    def add(text):
        buf = ctypes.create_unicode_buffer(text)
        user32.SendMessageW(box["list"], 0x0180, 0, ctypes.addressof(buf))

    def proc(hwnd, msg, wp, lp):
        if msg == 0x0111 and (wp & 0xFFFF) == 1:
            buf = ctypes.create_unicode_buffer(512)
            user32.GetWindowTextW(box["edit"], buf, 512)
            if buf.value.strip():
                add("me: " + buf.value)
                box["n"] = len(buf.value)
                user32.SetWindowTextW(box["edit"], "")
                user32.SetTimer(hwnd, 7, 300, None)
            return 0
        if msg == 0x0113:
            user32.KillTimer(hwnd, 7)
            add("peer: ok, %d chars" % box.get("n", 0))
            return 0
        if msg == 0x0002:
            user32.PostQuitMessage(0)
            return 0
        return user32.DefWindowProcW(hwnd, msg, wp, lp)

    wndproc = WNDPROC(proc)
    wc = WNDCLASSW(0, wndproc, 0, 0, inst, None, user32.LoadCursorW(None, 32512), 16, None, "LightingTestChat")
    user32.RegisterClassW(ctypes.byref(wc))
    hwnd = user32.CreateWindowExW(0, "LightingTestChat", "Lighting Test Chat", 0x10CF0000, 260, 240, 420, 360,
                                  None, None, inst, None)
    font = gdi32.GetStockObject(17)

    def make(cls, text, style, x, y, w, h, cid):
        c = user32.CreateWindowExW(0x200 if cls in ("EDIT", "LISTBOX") else 0, cls, text, 0x50000000 | style, x, y, w, h,
                                   hwnd, cid, inst, None)
        user32.SendMessageW(c, 0x0030, font, 1)
        return c

    make("STATIC", "Nachrichten in Test", 0, 16, 10, 300, 20, 0)
    box["list"] = make("LISTBOX", "", 0x00200001 | 0x00010000, 16, 32, 370, 200, 200)
    make("STATIC", "Nachricht an Test:", 0, 16, 244, 300, 20, 0)
    box["edit"] = make("EDIT", "", 0x00010080, 16, 266, 370, 26, 201)
    add("peer: hi, welcome")
    user32.ShowWindow(hwnd, 5)
    msg = W.MSG()
    while user32.GetMessageW(ctypes.byref(msg), None, 0, 0) > 0:
        if not user32.IsDialogMessageW(hwnd, ctypes.byref(msg)):
            user32.TranslateMessage(ctypes.byref(msg))
            user32.DispatchMessageW(ctypes.byref(msg))


if __name__ == "__main__":
    import sys
    chat() if sys.argv[1:] == ["chat"] else main()

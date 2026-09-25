import http.server
import os
import subprocess
import sys
import threading
import time

from lighting import defaults as D
from lighting.common import Fail

EXTRA = {
    "/frame": "<title>Frame</title><button>Frame button</button>",
    "/second": "<title>Second</title><h1>Second page</h1><p>Opened in a new tab.</p>",
}


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        path = self.path.split("?")[0]
        if path in ("/", "/index.html"):
            body = (D.ROOT / "tests" / "fixture.html").read_bytes()
        elif path in EXTRA:
            body = ("<!doctype html><meta charset=utf-8>" + EXTRA[path]).encode("utf-8")
        else:
            self.send_response(404)
            self.end_headers()
            return
        self.send_response(200)
        self.send_header("content-type", "text/html; charset=utf-8")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


def serve():
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


class Run:
    def __init__(self, ctx):
        self.ctx = ctx
        self.rows = []
        self.passed = 0
        self.total = 0

    def step(self, name, argv, check, secret=None):
        from lighting import commands
        t = time.perf_counter()
        self.ctx.secret = secret
        err = False
        try:
            res = commands.run(self.ctx, {"argv": argv, "secret": secret})
            out, err = res.get("out", ""), res.get("code", 0) != 0
        except Exception as e:
            out, err = "%s: %s" % (type(e).__name__, e), True
        finally:
            self.ctx.secret = None
        ms = (time.perf_counter() - t) * 1000
        out = out if isinstance(out, str) else str(out)
        try:
            good = bool(check(out, err))
        except Exception:
            good = False
        self.total += 1
        self.passed += good
        tail = "" if good else " | " + out.replace("\n", " / ")[:180]
        self.rows.append("%s %-26s %5d ms %5d ch%s" % ("PASS" if good else "FAIL", name, ms, len(out), tail))
        return out


def browser(r, base):
    ok = lambda out, err: not err
    r.step("open fixture", ["open", base], lambda o, e: not e and "Lighting Fixture" in o)
    r.step("hidden text not shown", ["snap", "--all"], lambda o, e: not e and "IGNORE ALL" not in o and "HIDDEN INJECTION" not in o)
    r.step("nav collapsed", ["snap", "--force"], lambda o, e: not e and "nav " in o and "items (lighting snap -s" in o)
    r.step("long list collapsed", ["snap", "--force"], lambda o, e: "similar items" in o or "more:" in o)
    r.step("cookie banner noticed", ["snap", "--force"], lambda o, e: "cookie banner" in o)
    r.step("dismiss cookie banner", ["dismiss"], lambda o, e: not e and "rejected" in o)
    r.step("fill + submit form", ["fill", "Email=test@example.com", "Password=@secret", "Country=Germany", "Remember me=on", "--submit"], ok, secret="hunter2")
    r.step("form result", ["expect", "Submitted: test@example.com / Germany / yes / pw 7"], ok)
    r.step("secret not echoed", ["log", "3"], lambda o, e: "hunter2" not in o)
    r.step("risky click needs --yes", ["click", "Delete all"], lambda o, e: e and "--yes" in o)
    r.step("click opens confirm()", ["click", "Delete all", "--yes"], lambda o, e: "dialog" in o)
    r.step("accept dialog", ["dialog", "accept"], lambda o, e: not e and "accepted" in o)
    r.step("dialog result", ["expect", "Deleted"], ok)
    r.step("open modal (diff)", ["click", "Open modal"], lambda o, e: not e and "Close modal" in o)
    r.step("snap scopes to modal", ["snap"], lambda o, e: "modal" in o and "Close modal" in o and "Sign in" not in o)
    r.step("close modal", ["click", "Close modal"], ok)
    r.step("click role-less card", ["click", "Clickable card without role"], ok)
    r.step("card clicked", ["expect", "Card clicked"], ok)
    r.step("wait for late button", ["wait", "Appears later", "--timeout", "5000"], ok)
    r.step("table as tsv", ["table", "#prices"], lambda o, e: not e and "Beta\t2.50" in o)
    r.step("type contenteditable", ["type", "Notes", "hello from lighting"], ok)
    r.step("notes text present", ["expect", "hello from lighting"], ok)
    r.step("text (markdown)", ["text"], lambda o, e: not e and "Lighting Fixture" in o and "IGNORE ALL" not in o)
    r.step("js", ["js", "document.title"], lambda o, e: o.strip() == "Lighting Fixture")
    r.step("fetch json-less", ["fetch", base + "frame"], lambda o, e: not e and "200" in o)
    r.step("snap iframe", ["snap", "--frame", "127.0.0.1"], lambda o, e: not e and "Frame button" in o)
    r.step("shot element", ["shot", "e1"], lambda o, e: not e and ".jpg" in o)
    r.step("new tab event", ["click", "Open in new tab"], lambda o, e: not e and "new tab t" in o)
    r.step("close popup tab", ["close"], ok)
    r.step("read url without browser", ["read", base], lambda o, e: not e and "Lighting Fixture" in o and "IGNORE" not in o)
    r.step("unchanged snap", ["snap"], ok)
    r.step("blocklist enforced", ["_blocked"], lambda o, e: e and "blocked" in o)
    r.step("close fixture tab", ["close"], ok)


def close_apps(win):
    for w in win.windows():
        if w["title"] == "Lighting Test App":
            win.user32.PostMessageW(w["hwnd"], 0x0010, 0, 0)
    deadline = time.time() + 3
    while time.time() < deadline and any(w["title"] == "Lighting Test App" for w in win.windows()):
        time.sleep(0.05)


def desktop(r):
    from lighting import desktop as dk
    from lighting import win
    close_apps(win)
    env = dict(os.environ, PYTHONPATH=str(D.ROOT))
    exe = str(D.PYW if D.PYW.exists() else sys.executable)
    subprocess.Popen([exe, "-m", "lighting.testapp"], env=env)
    try:
        deadline = time.time() + 8
        while time.time() < deadline and not any(w["title"] == "Lighting Test App" for w in win.windows()):
            time.sleep(0.1)
        before = win.cursor()
        out = r.step("snap test app", ["snap", "app:Lighting Test App"], lambda o, e: not e and "edit" in o and "checkbox" in o)
        refs = {}
        for line in out.splitlines():
            bits = line.split(" ", 2)
            if len(bits) >= 2 and bits[0].startswith("d"):
                refs.setdefault(bits[1], bits[0])
        r.step("type (background)", ["type", refs.get("edit", "d0"), "Claude"], lambda o, e: not e and "background" in o)
        r.step("toggle checkbox", ["click", refs.get("checkbox", "d0")], lambda o, e: not e)
        r.step("click button by name", ["click", "Go"], lambda o, e: not e)
        r.step("read status via uia", ["snap", "app:Lighting Test App", "--text", "-f", "status"], lambda o, e: "clicked 1 with Claude" in o)
        r.step("ocr read", ["read", "app:Lighting Test App"], lambda o, e: not e and "Remember" in o)
        after = win.cursor()
        r.step("mouse untouched", ["ping"], lambda o, e: before == after)
        r.step("window shot", ["shot", "app:Lighting Test App"], lambda o, e: not e and ".jpg" in o)
    finally:
        close_apps(win)
        dk.state(r.ctx).hwnd = None
        r.ctx.target = None


def run(ctx, pos, flags):
    from lighting import browser as web
    from lighting import commands
    r = Run(ctx)
    srv = None
    only = pos[0] if pos else "all"
    original = commands.SYSTEM.get("_blocked")

    def blocked(c, p, f):
        host = web.ensure_host(c)
        payload = web.config_payload(c.cfg)
        payload["blocklist"] = payload["blocklist"] + ["127.0.0.1"]
        host.post("config", payload)
        try:
            return web.cmd_click(c, ["Sign in"], {})
        finally:
            web.push_config(c)

    commands.SYSTEM["_blocked"] = blocked
    try:
        if only in ("all", "web") and web.any_browser(ctx):
            srv = serve()
            browser(r, "http://127.0.0.1:%d/" % srv.server_address[1])
        elif only in ("all", "web"):
            r.rows.append("SKIP browser tests (no browser connected: lighting setup)")
        if only in ("all", "app"):
            desktop(r)
    finally:
        if original is None:
            commands.SYSTEM.pop("_blocked", None)
        if srv:
            srv.shutdown()
    r.rows.append("%d/%d passed" % (r.passed, r.total))
    return "\n".join(r.rows)


def bench(ctx, pos, flags):
    from lighting import commands
    srv = serve()
    base = "http://127.0.0.1:%d/" % srv.server_address[1]
    rows = ["command                     ms   chars  ~tokens"]
    plan = [["open", base], ["snap", "--force"], ["snap", "-f", "email"], ["click", "Open modal"], ["click", "Close modal"],
            ["text"], ["read", base], ["tabs"], ["close"]]
    try:
        for argv in plan:
            t = time.perf_counter()
            try:
                out, _, _ = commands.run_one(ctx, argv)
            except Fail as e:
                out = e.text()
            ms = (time.perf_counter() - t) * 1000
            rows.append("%-26s %5d %6d %7d" % (" ".join(argv).replace(base, "<fixture>")[:26], ms, len(out), len(out) // 4 + 1))
    finally:
        srv.shutdown()
    return "\n".join(rows)

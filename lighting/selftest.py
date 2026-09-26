import http.server
import os
import re
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


def make_pdf(text):
    stream = b"BT /F1 18 Tf 20 40 Td (" + text + b") Tj ET"
    objs = [b"<</Type/Catalog/Pages 2 0 R>>", b"<</Type/Pages/Kids[3 0 R]/Count 1>>",
            b"<</Type/Page/Parent 2 0 R/MediaBox[0 0 300 100]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>>>>>",
            b"<</Length %d>>stream\n" % len(stream) + stream + b"\nendstream",
            b"<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>"]
    out, offsets = b"%PDF-1.4\n", []
    for i, obj in enumerate(objs, 1):
        offsets.append(len(out))
        out += b"%d 0 obj" % i + obj + b"endobj\n"
    xref = len(out)
    out += b"xref\n0 %d\n0000000000 65535 f \n" % (len(objs) + 1)
    out += b"".join(b"%010d 00000 n \n" % o for o in offsets)
    return out + b"trailer<</Size %d/Root 1 0 R>>\nstartxref\n%d\n%%%%EOF\n" % (len(objs) + 1, xref)


PDF = make_pdf(b"Hello Lighting PDF")


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        path = self.path.split("?")[0]
        ctype = "text/html; charset=utf-8"
        if path in ("/", "/index.html"):
            body = (D.ROOT / "tests" / "fixture.html").read_bytes()
        elif path == "/doc.pdf":
            body, ctype = PDF, "application/pdf"
        elif path in EXTRA:
            body = ("<!doctype html><meta charset=utf-8>" + EXTRA[path]).encode("utf-8")
        else:
            self.send_response(404)
            self.end_headers()
            return
        self.send_response(200)
        self.send_header("content-type", ctype)
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

    def fn(self, name, func):
        t = time.perf_counter()
        try:
            good, out = func()
        except Exception as e:
            good, out = False, "%s: %s" % (type(e).__name__, e)
        self.total += 1
        self.passed += bool(good)
        tail = "" if good else " | " + str(out).replace("\n", " / ")[:180]
        self.rows.append("%s %-26s %5d ms %5d ch%s" % ("PASS" if good else "FAIL", name, (time.perf_counter() - t) * 1000, len(str(out)), tail))


def printwindow_ocr():
    from lighting import ocr
    from lighting import win
    hwnd = next(w["hwnd"] for w in win.windows() if w["title"] == "Lighting Test App")
    img = win.print_window(hwnd)
    if img is None:
        return False, "PrintWindow returned nothing"
    text = " ".join(t for t, *_ in ocr.recognize(img, "en"))
    return "Remember" in text, text


def tab_ref(out):
    m = re.match(r"\[(t\d+)\]", out)
    return m.group(1) if m else "t0"


def browser(r, base):
    ok = lambda out, err: not err
    decoy = r.step("decoy tab", ["open", base + "second", "--new"], lambda o, e: not e and "Second" in o)
    r.step("open fixture", ["open", base, "--new"], lambda o, e: not e and "Lighting Fixture" in o)
    r.step("open with a filter", ["open", base, "--new", "-f", "Sign in"],
           lambda o, e: not e and "Sign in" in o and "Dark mode" not in o and "nav " not in o)
    r.step("close filtered tab", ["close"], lambda out, err: not err)
    r.step("hidden text not shown", ["snap", "--all"], lambda o, e: not e and "IGNORE ALL" not in o and "HIDDEN INJECTION" not in o)
    r.step("nav collapsed", ["snap", "--force"], lambda o, e: not e and "nav " in o and "items (lighting snap -s" in o)
    r.step("header collapsed, names kept", ["snap", "--force"],
           lambda o, e: not e and re.search(r"header: 13 items \(lighting snap -s e\d+\): Top 1 \| Top 2", o) and 'link "Top 5"' not in o)
    r.step("footnotes, key hints, hashes", ["snap", "--all"],
           lambda o, e: not e and '"[1]"' not in o and 'link "Keyboard help" ->' in o and "commit/0123456…" in o)
    r.step("long list collapsed", ["snap", "--force"], lambda o, e: "similar items" in o or "more:" in o)
    r.step("cookie banner noticed", ["snap", "--force"], lambda o, e: "cookie banner" in o)
    r.step("dismiss cookie banner", ["dismiss"], lambda o, e: not e and "rejected" in o)
    r.step("toggle shows [ ]", ["snap", "-f", "Dark mode"], lambda o, e: not e and '"Dark mode" [ ]' in o)
    r.step("toggle click", ["click", "Dark mode"], ok)
    r.step("toggle shows [x]", ["snap", "-f", "Dark mode"], lambda o, e: not e and '"Dark mode" [x]' in o)
    r.step("filter a|b", ["snap", "-f", "Sign in|Open modal"], lambda o, e: not e and "Sign in" in o and "Open modal" in o)
    r.step("nameless button hint", ["snap", "--all"], lambda o, e: not e and "button #nameless-save" in o)
    r.step("click by #hint", ["click", "#nameless-save"], ok)
    r.step("hint click worked", ["expect", "Saved via hint"], ok)
    vout = r.step("video line", ["snap", "-f", "video"], lambda o, e: not e and 'video "demo clip" 0:00' in o and "paused" in o)
    found = re.search(r"^(e\d+) video", vout, re.M)
    vref = found.group(1) if found else "e0"
    r.step("images with --media", ["snap", "--media", "-f", "orange test square"],
           lambda o, e: not e and 'img "orange test square" 64x64' in o)
    r.step("frames of a paused video", ["frames", vref, "--count", "2", "--every", "400"],
           lambda o, e: not e and ".jpg" in o and "did not move" in o and "not seekable" in o)
    r.step("frames read captions", ["frames", vref, "--count", "2", "--every", "300"],
           lambda o, e: not e and "captions:" in o and "Hello from the caption track" in o)
    from PIL import Image
    pic = D.OUT / "selftest.png"
    Image.new("RGB", (40, 20), "orange").save(pic)
    r.step("read an image file", ["read", str(pic)], lambda o, e: not e and "(image)" in o and "40x20" in o)
    r.step("page changes by itself", ["js", "setTimeout(() => history.pushState({}, '', '?moved=1'), 50); 1"],
           lambda o, e: (time.sleep(0.5) or True) and not e)
    r.step("stale text click refused", ["click", "#nameless-save"], lambda o, e: e and "page changed" in o)
    r.step("next click works", ["click", "#nameless-save"], ok)
    script = D.OUT / "selftest.js"
    script.write_text("return document.title", "utf-8")
    r.step("js --file", ["js", "--file", str(script)], lambda o, e: o.strip() == "Lighting Fixture")
    r.step("unknown tab alias", ["tab", "t9999"], lambda o, e: e and "no tab t9999" in o)
    r.step("viewport 390x844", ["viewport", "390x844"], lambda o, e: not e and "mobile" in o)
    r.step("viewport applied", ["js", "innerWidth"], lambda o, e: o.strip() == "390")
    r.step("viewport reset", ["viewport", "reset"], ok)
    r.step("viewport restored", ["js", "innerWidth"], lambda o, e: o.strip().isdigit() and o.strip() != "390")
    r.step("shot --marks", ["shot", "--marks"], lambda o, e: not e and ".jpg" in o and re.search(r"\b[1-9]\d* orange labels", o))
    r.step("marks removed", ["js", "document.querySelectorAll('lt-pointer[data-marks]').length"], lambda o, e: o.strip() == "0")
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
    r.step("text (markdown)", ["text"], lambda o, e: not e and "| Beta | 2.50 |" in o and "IGNORE ALL" not in o and "<iframe" not in o and "readable view" not in o)
    r.step("js", ["js", "document.title"], lambda o, e: o.strip() == "Lighting Fixture")
    r.step("js with const", ["js", "const v = 2; v * 3"], lambda o, e: o.strip() == "6")
    r.step("js with const again", ["js", "const v = 2; v * 3"], lambda o, e: o.strip() == "6")
    r.step("fetch json-less", ["fetch", base + "frame"], lambda o, e: not e and "200" in o)
    r.step("snap iframe", ["snap", "--frame", "127.0.0.1"], lambda o, e: not e and "Frame button" in o)
    r.step("shot element", ["shot", "e1"], lambda o, e: not e and ".jpg" in o)
    r.step("new tab event", ["click", "Open in new tab"], lambda o, e: not e and "new tab t" in o)
    r.step("close popup tab", ["close"], ok)
    r.step("read url without browser", ["read", base], lambda o, e: not e and "Lighting Fixture" in o and "IGNORE" not in o)
    r.step("read 2 urls in parallel", ["read", base, base + "second"], lambda o, e: not e and "Lighting Fixture" in o and "Opened in a new tab" in o)
    r.step("read pdf as text", ["read", base + "doc.pdf"], lambda o, e: not e and "Hello Lighting PDF" in o and "pdf, 1 pages" in o)
    leak = "http://leak-%d.invalid/?d=%s" % (int(time.time() * 1000), "x" * 300)
    r.step("leak guard blocks", ["read", leak], lambda o, e: e and "possible data leak" in o)
    r.step("leak guard --yes passes", ["read", leak, "--yes"], lambda o, e: e and "read failed" in o)
    r.step("unchanged snap", ["snap"], ok)
    r.step("blocklist enforced", ["_blocked"], lambda o, e: e and "blocked" in o)
    r.step("record start (web)", ["record", "start", "selftest-rec", "--web"], lambda o, e: not e and "recording" in o)
    r.step("recorded fill", ["fill", "Email=rec@example.com"], ok)
    r.step("recorded click", ["click", "Dark mode"], ok)
    r.step("record stop (web)", ["record", "stop", "email=rec@example.com"],
           lambda o, e: not e and "fill Email={email}" in o and 'click "Dark mode"' in o)
    r.step("run recorded routine", ["run", "selftest-rec", "email=run@example.com"], lambda o, e: not e and o.startswith("ok selftest-rec"))
    r.step("routine filled field", ["js", "document.getElementById('email').value"], lambda o, e: o.strip() == "run@example.com")
    r.step("remove web routine", ["routine", "rm", "selftest-rec"], ok)
    r.step("close fixture tab", ["close"], ok)
    r.step("target back to decoy", ["snap"], lambda o, e: not e and "Second" in o)
    dref = tab_ref(decoy)
    listed = lambda o, ref, lit: re.search(r"^%s %s" % (ref, "L[*a]* " if lit else "(?!L[*a]* )"), o, re.M)
    r.step("keep hands tab over", ["keep", dref], lambda o, e: not e and "kept 1 tab" in o)
    r.step("kept tab left the group", ["tabs"], lambda o, e: listed(o, dref, False))
    tref = tab_ref(r.step("throwaway tab", ["open", base + "second", "--new"], lambda o, e: not e and "Second" in o))
    from lighting import commands
    from lighting import desktop as dk
    lit = re.findall(r"^(t\d+) L[*a]* ", commands.run(r.ctx, {"argv": ["tabs"]}).get("out", ""), re.M)
    if [t for t in lit if t != tref] or dk.state(r.ctx).launched:
        r.rows.append("SKIP done closes tabs (other Lighting tabs or launched apps are open)")
        r.step("close throwaway tab", ["close", tref], ok)
    else:
        r.step("done closes tabs", ["done"], lambda o, e: not e and o.startswith("closed 1 tab"))
        r.step("done kept the kept tab", ["tabs"], lambda o, e: not listed(o, tref, True) and listed(o, dref, False))
    r.step("close kept tab", ["close", dref, "--force"], ok)


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
        before, t_before = win.cursor(), time.time()
        out = r.step("snap test app", ["snap", "app:Lighting Test App"], lambda o, e: not e and "edit" in o and "checkbox" in o)
        refs = {}
        for line in out.splitlines():
            bits = line.split(" ", 2)
            if len(bits) >= 2 and bits[0].startswith("d"):
                refs.setdefault(bits[1], bits[0])
        r.step("type by field name", ["type", "Name:", "Claude"], lambda o, e: not e and "background" in o)
        r.step("toggle checkbox", ["click", refs.get("checkbox", "d0")], lambda o, e: not e)
        r.step("click button by name", ["click", "Go"], lambda o, e: not e)
        r.step("read status via uia", ["snap", "app:Lighting Test App", "--text", "-f", "status"], lambda o, e: "clicked 1 with Claude" in o)
        r.step("ocr read", ["read", "app:Lighting Test App"], lambda o, e: not e and "Remember" in o)
        r.fn("ocr via PrintWindow", printwindow_ocr)
        after = win.cursor()
        if before != after and win.idle_ms() < (time.time() - t_before) * 1000:
            r.rows.append("SKIP mouse untouched (you used mouse or keyboard during the test)")
        else:
            r.step("mouse untouched", ["ping"], lambda o, e: before == after)
        r.step("record start (app)", ["record", "start", "selftest-app", "--desktop", "--injected"], lambda o, e: not e)
        r.step("recorded app click", ["click", "Go", "--mouse"], lambda o, e: not e)
        r.step("record stop (app)", ["record", "stop"], lambda o, e: not e and "click Go" in o)
        r.step("remove app routine", ["routine", "rm", "selftest-app"], lambda o, e: not e)
        r.step("window shot", ["shot", "app:Lighting Test App"], lambda o, e: not e and ".jpg" in o)
        wref = re.match(r"\[(w\d+)\]", out)
        r.step("close app window", ["close", wref.group(1) if wref else "w0"], lambda o, e: not e and "closed" in o)
    finally:
        close_apps(win)
        dk.state(r.ctx).hwnd = None
        r.ctx.target = None


def tiers_check():
    from lighting.common import best_only, terms, tiers
    names = ["SOS", "Sosa La M", "SOS wiedergeben", "commits", "MIT license"]
    kept = best_only([(t, n) for t, n in ((tiers(terms("sos|mit"), n), n) for n in names) if any(t)])
    return kept == ["SOS", "SOS wiedergeben", "MIT license"], kept


def learn_check():
    from lighting import routines as R
    st = lambda c, *a: {"cmd": c, "args": list(a), "flags": {}, "where": None}
    A = [st("launch", "spotify:search:SOS"), st("click", "SOS wiedergeben")]
    B = [st("launch", "spotify:search:Numb"), st("click", "Numb wiedergeben")]
    body = R.build(A, {"steps": B, "start": None}, R.align(A, B))
    lines = [R.step_line(s) for s in body["steps"]]
    return body["params"] == {"search": "Numb"} and lines == ["launch spotify:search:{search}", 'click "{search} wiedergeben"'], lines


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
    r.fn("filter tiers", tiers_check)
    r.fn("routine learning", learn_check)
    learn = ctx.no_learn
    ctx.no_learn = True
    try:
        if only in ("all", "web") and web.any_browser(ctx):
            srv = serve()
            browser(r, "http://127.0.0.1:%d/" % srv.server_address[1])
        elif only in ("all", "web"):
            r.rows.append("SKIP browser tests (no browser connected: lighting setup)")
        if only in ("all", "app"):
            desktop(r)
    finally:
        ctx.no_learn = learn
        if original is None:
            commands.SYSTEM.pop("_blocked", None)
        if srv:
            srv.shutdown()
    r.rows.append("%d/%d passed" % (r.passed, r.total))
    return "\n".join(r.rows)


REAL_SITES = ["github.com/QDHShamiro/Lighting", "en.wikipedia.org/wiki/Minecraft", "www.youtube.com",
              "modrinth.com/plugins", "huggingface.co/models", "www.tiktok.com/foryou"]


def bench_real(ctx, pos, flags):
    import json
    from lighting import commands
    sites = [p for p in pos if p != "real"] or REAL_SITES
    path = D.HOME / "bench-real.json"
    try:
        before = json.loads(path.read_text("utf-8"))
    except (OSError, ValueError):
        before = {}
    now, rows = {}, ["site                          open ms  tok   snap ms  tok   text ms   tok  raw page tok  saved  vs last"]
    total = [0, 0]
    learn = ctx.no_learn
    ctx.no_learn = True
    try:
        for i, site in enumerate(sites):
            res = {}
            for key, argv in (("open", ["open", site] + (["--new"] if i == 0 else [])), ("snap", ["snap", "--force"]), ("text", ["text"])):
                t = time.perf_counter()
                try:
                    out, _, _ = commands.run_one(ctx, argv)
                except Fail as e:
                    out = e.text()
                res[key] = [int((time.perf_counter() - t) * 1000), len(out) // 4 + 1]
            try:
                raw = int(commands.run_one(ctx, ["js", "document.body.innerText.length"])[0].strip()) // 4 + 1
            except (Fail, ValueError):
                raw = 0
            now[site] = dict(res, raw=[0, raw])
            if raw >= 50:
                total[0] += res["text"][1]
                total[1] += raw
            old = before.get(site)
            delta = ""
            if old:
                a = sum(old[k][1] for k in ("open", "snap", "text") if k in old)
                b = sum(res[k][1] for k in ("open", "snap", "text"))
                delta = "%+d%% tok" % round((b - a) * 100.0 / max(1, a))
            saved = "%d%%" % round(100 - res["text"][1] * 100.0 / raw) if raw >= 50 else "-"
            rows.append("%-29s %6d %5d %8d %5d %8d %6d %13d %6s  %s" % (
                site[:29], res["open"][0], res["open"][1], res["snap"][0], res["snap"][1], res["text"][0], res["text"][1],
                raw, saved, delta))
        if total[1]:
            rows.append("total: text %d tokens vs raw page text %d tokens = %d%% saved" % (
                total[0], total[1], round(100 - total[0] * 100.0 / total[1])))
        try:
            commands.run_one(ctx, ["close"])
        except Fail:
            pass
    finally:
        ctx.no_learn = learn
    path.write_text(json.dumps(now, indent=1), "utf-8")
    return "\n".join(rows)


def bench(ctx, pos, flags):
    from lighting import commands
    if flags.get("real") or (pos and pos[0] == "real"):
        return bench_real(ctx, pos, flags)
    srv = serve()
    base = "http://127.0.0.1:%d/" % srv.server_address[1]
    rows = ["command                     ms   chars  ~tokens"]
    plan = [["open", base, "--new"], ["snap", "--force"], ["snap", "-f", "email"], ["click", "Open modal"], ["click", "Close modal"],
            ["text"], ["read", base], ["tabs"], ["close"]]
    learn = ctx.no_learn
    ctx.no_learn = True
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
        ctx.no_learn = learn
        srv.shutdown()
    return "\n".join(rows)

import http.server
import threading
import time

from lighting import defaults as D
from lighting.common import Fail

PAGE = D.ROOT / "lighting" / "demo.html"
PLACE = "<demo page>"


def steps(base):
    return [["open", base, "--new"],
            ["snap", "-f", "name|email|plan|terms|trial"],
            ["fill", "Name=Ada Lovelace", "Email=ada@example.com"],
            ["select", "Plan", "Pro"],
            ["check", "I accept the terms"],
            ["click", "Start free trial", "-f", "Welcome"],
            ["table", "table"]]


class Handler(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path.split("?")[0] not in ("/", "/index.html"):
            self.send_response(404)
            self.end_headers()
            return
        body = PAGE.read_bytes()
        self.send_response(200)
        self.send_header("content-type", "text/html; charset=utf-8")
        self.send_header("content-length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


def shown(argv, base):
    return " ".join('"%s"' % a if " " in a else a for a in argv).replace(base, PLACE)


def run(ctx, pos, flags):
    from lighting import browser, commands
    if not browser.any_browser(ctx):
        raise Fail("the demo needs the browser", "lighting setup")
    pause = 0.0 if flags.get("fast") else float(flags.get("pause") or D.DEMO_PAUSE_S)
    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    base = "http://127.0.0.1:%d/" % srv.server_address[1]
    blocks, work, chars, failed = [], 0.0, 0, 0
    learn = ctx.no_learn
    ctx.no_learn = True
    start = time.perf_counter()
    try:
        for i, argv in enumerate(steps(base)):
            if i and pause:
                time.sleep(pause)
            t = time.perf_counter()
            try:
                out, _, _ = commands.run_one(ctx, argv)
            except Fail as e:
                out, failed = e.text(), failed + 1
            ms = (time.perf_counter() - t) * 1000
            work += ms
            out = (out if isinstance(out, str) else str(out)).replace(base, PLACE).strip()
            chars += len(out)
            blocks.append("$ lighting %s   (%d ms, ~%d tokens)\n%s" % (shown(argv, base), ms, len(out) // 4 + 1, out))
        try:
            raw = int(commands.run_one(ctx, ["js", "document.body.innerText.length"])[0].strip()) // 4 + 1
        except (Fail, ValueError):
            raw = 0
    finally:
        ctx.no_learn = learn
        srv.shutdown()
    total = time.perf_counter() - start
    head = "Lighting demo: %d commands on a made-up shop page, %.1f s%s" % (
        len(blocks), total, " with pauses (--fast skips them)" if pause else "")
    tail = "%d commands, %.1f s of work, ~%d tokens of answers in all" % (len(blocks), work / 1000, chars // 4 + 1)
    if raw:
        tail += "; the page itself is ~%d tokens of text" % raw
    if failed:
        tail += "; %d failed (lighting doctor)" % failed
    return "\n\n".join([head] + blocks + [tail])

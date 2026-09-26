import json
import os
import shlex
import sys
import time

from lighting import defaults as D
from lighting.common import Fail, cap, is_url, ref_kind

VALUED = {"f", "s", "d", "max", "browser", "lang", "timeout", "width", "until", "pick", "frame",
          "body", "method", "button", "region", "delta", "file", "role", "last", "on", "count", "every", "from", "to"}
TRACE = (D.HOME / "trace").exists()
ALIAS = {"-f": "f", "-s": "s", "-d": "d", "-n": "new", "-a": "all", "-y": "yes", "-e": "errors", "-g": "gone",
         "-m": "media"}


class Context:
    def __init__(self, daemon):
        self.d = daemon
        self.cfg = D.config()
        self.target = None
        self.secret = None
        self.cwd = None
        self.image = None
        self.windows = {}
        self.last_shot = {}
        self.last_target = None
        self.where = None
        self.no_learn = False

    def kind(self):
        return self.target[0] if self.target else None


def parse(argv):
    name = argv[0].lower() if argv else ""
    pos, flags, i = [], {}, 1
    while i < len(argv):
        a = argv[i]
        key = None
        if a.startswith("--") and len(a) > 2:
            key, _, val = a[2:].partition("=")
            if _:
                flags[key] = val
                i += 1
                continue
        elif a in ALIAS:
            key = ALIAS[a]
        if key is None:
            pos.append(a)
        elif key in VALUED and i + 1 < len(argv):
            flags[key] = argv[i + 1]
            i += 1
        else:
            flags[key] = True
        i += 1
    return name, pos, flags


def split_steps(text):
    steps, cur, quote = [], [], None
    for ch in text:
        if quote:
            if ch == quote:
                quote = None
            cur.append(ch)
        elif ch in "\"'":
            quote = ch
            cur.append(ch)
        elif ch == ";":
            steps.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
    steps.append("".join(cur))
    return [s.strip() for s in steps if s.strip()]


def tokenize(step):
    lex = shlex.shlex(step, posix=True)
    lex.whitespace_split = True
    lex.escape = ""
    lex.commenters = ""
    return list(lex)


def web():
    from lighting import browser
    return browser


def app():
    from lighting import desktop
    return desktop


def side(ctx, pos, verb, flags=None):
    on = (flags or {}).get("on")
    if on == "web":
        return web()
    if on == "app":
        return app()
    kind = ref_kind(pos[0]) if pos else None
    if kind in ("e", "t"):
        return web()
    if kind in ("d", "o", "w"):
        return app()
    if pos and (pos[0] == "screen" or pos[0].lower().startswith("app:")):
        return app()
    if ctx.kind() == "app":
        return app()
    if ctx.kind() == "web":
        return web()
    if verb in ("snap", "click", "type", "press", "scroll", "hover", "drag", "shot") and not web().any_browser(ctx):
        return app()
    return web()


def route(ctx, name, pos, flags):
    if name in SYSTEM:
        return SYSTEM[name](ctx, pos, flags)
    if name == "read":
        if pos and (is_url(pos[0]) or pos[0].lower().endswith((".pdf",) + web().IMAGE_EXT)):
            return web().read_url(ctx, pos, flags)
        return app().cmd_read(ctx, pos, flags)
    if name == "close" and pos and (ref_kind(pos[0]) == "w" or pos[0].lower().startswith("app:")):
        return app().cmd_close(ctx, pos, flags)
    if name in APP_ONLY:
        return getattr(app(), "cmd_" + name)(ctx, pos, flags)
    if name in SHARED:
        return getattr(side(ctx, pos, name, flags), "cmd_" + name)(ctx, pos, flags)
    if name in WEB_ONLY:
        return getattr(web(), "cmd_" + name)(ctx, pos, flags)
    raise Fail("unknown command '%s'" % name, "lighting help")


def run_one(ctx, argv):
    from lighting import routines
    name, pos, flags = parse(argv)
    if not name:
        raise Fail("no command", "lighting help")
    ctx.last_target = None
    if name in SYSTEM:
        return route(ctx, name, pos, flags), name, pos
    try:
        t0 = time.perf_counter()
        out = route(ctx, name, pos, flags)
        t1 = time.perf_counter()
    except Fail as e:
        routines.observe(ctx, name, pos, flags, e.text(), False)
        raise
    routines.observe(ctx, name, pos, flags, out if isinstance(out, str) else "", True)
    if TRACE:
        print("trace %s: route %.2f ms, observe %.2f ms" % (name, (t1 - t0) * 1000, (time.perf_counter() - t1) * 1000),
              file=sys.stderr, flush=True)
    return out, name, pos


def run(ctx, msg):
    ctx.secret = msg.get("secret")
    ctx.cwd = msg.get("cwd")
    ctx.image = None
    argv = msg.get("argv") or []
    t = time.perf_counter()
    code = 0
    name = argv[0] if argv else ""
    try:
        out, name, pos = run_one(ctx, argv)
    except Fail as e:
        out, code = e.text(), 1
    except TimeoutError as e:
        out, code = "err: " + str(e), 1
    finally:
        ctx.secret = None
    if isinstance(out, dict):
        return out
    events = []
    while ctx.d.events:
        ev = ctx.d.events.popleft()
        quoted = ev.split('"')[1] if ev.count('"') >= 2 else None
        if quoted and '"%s"' % quoted in out:
            continue
        events.append("! " + ev)
    if events:
        out = (out + "\n" if out else "") + "\n".join(events)
    t_log = time.perf_counter()
    log(name, argv, code, (time.perf_counter() - t) * 1000)
    if TRACE:
        print("trace %s: total %.2f ms, log %.2f ms" % (name, (time.perf_counter() - t) * 1000, (time.perf_counter() - t_log) * 1000),
              file=sys.stderr, flush=True)
    res = {"out": out, "code": code}
    if ctx.image:
        res["image"] = ctx.image
    return res


_log = {"f": None, "n": 0}


def log(name, argv, code, ms):
    if name in ("ping", "status", "log"):
        return
    try:
        if _log["f"] is None or _log["n"] >= 500:
            if _log["f"]:
                _log["f"].close()
                _log["f"] = None
            if D.LOG.exists() and D.LOG.stat().st_size > D.LOG_MAX_BYTES:
                D.LOG.replace(D.LOG.with_suffix(".old.jsonl"))
            _log["f"], _log["n"] = open(D.LOG, "a", encoding="utf-8", buffering=1), 0
        _log["n"] += 1
        ref = next((a for a in argv[1:2] if ref_kind(a)), "")
        _log["f"].write(json.dumps({"t": time.strftime("%Y-%m-%d %H:%M:%S"), "cmd": name, "ref": ref,
                                    "ok": code == 0, "ms": int(ms)}) + "\n")
    except OSError:
        _log["f"] = None


def cmd_do(ctx, pos, flags):
    text = " ".join(pos)
    steps = split_steps(text)
    if not steps:
        raise Fail("do needs steps", 'lighting do "click e3; type e4 hi; press Enter"')
    lines = []
    for i, step in enumerate(steps, 1):
        if ctx.d.abort.is_set():
            lines.append("%d stopped by hotkey" % i)
            break
        argv = tokenize(step)
        try:
            out, _, _ = run_one(ctx, argv)
        except Fail as e:
            left = len(steps) - i
            lines.append("%d %s%s" % (i, e.text(), " (stopped, %d skipped)" % left if left else ""))
            break
        text = out if isinstance(out, str) else json.dumps(out)
        if i < len(steps) and parse(argv)[0] not in READS:
            text = text.split("\n", 1)[0]
        prefix = "%d " % i if len(steps) > 1 else ""
        lines.append(prefix + text)
    return "\n".join(lines)


def cmd_ping(ctx, pos, flags):
    return "pong"


def cmd_stop(ctx, pos, flags):
    from lighting import routines
    routines.close(ctx)
    return {"out": "lighting daemon stopped", "code": 0, "shutdown": True}


def plural(n, word):
    return "%d %s%s" % (n, word, "" if n == 1 else "s")


def browsers(ctx, cmd, args=None):
    replies = []
    for h in [h for h in ctx.d.hosts if h.alive]:
        try:
            replies.append(h.call(cmd, args or {}, timeout=5))
        except TimeoutError:
            pass
    return replies


def cmd_done(ctx, pos, flags):
    quiet = flags.get("quiet")
    if not ctx.cfg.get("cleanup"):
        browsers(ctx, "cleanup", {"close": False})
        return "" if quiet else "left everything open (cleanup is off: lighting config cleanup on)"
    replies = browsers(ctx, "cleanup")
    tabs = sum(r.get("closed") or 0 for r in replies)
    viewed = sum(r.get("kept") or 0 for r in replies)
    closed, left = app().cleanup(ctx)
    if quiet:
        return ""
    parts = ([plural(tabs, "tab")] if tabs else []) + (
        ["%s (%s)" % (plural(len(closed), "app"), ", ".join(closed))] if closed else [])
    lines = ["closed " + ", ".join(parts) if parts else "nothing to close"]
    if viewed:
        lines.append("left %s open that the user is looking at" % plural(viewed, "tab"))
    if left:
        lines.append("still open, it may ask to save: " + ", ".join(left))
    return "\n".join(lines)


def cmd_keep(ctx, pos, flags):
    spec = pos[0] if pos else None
    kind = ref_kind(spec) if spec else None
    tabs = apps = 0
    if not spec or kind == "t":
        tabs = sum(r.get("kept") or 0 for r in browsers(ctx, "keep", {"id": spec}))
    if not spec or kind == "w" or spec.lower().startswith("app:"):
        apps = app().keep(ctx, spec)
    parts = [plural(n, w) for n, w in ((tabs, "tab"), (apps, "app")) if n]
    if not parts:
        return "nothing to keep"
    return "kept %s: stays open after the task" % " and ".join(parts)


def ago(seconds):
    seconds = int(seconds)
    if seconds < 90:
        return "%ds" % seconds
    if seconds < 5400:
        return "%dm" % (seconds // 60)
    return "%dh" % (seconds // 3600)


def cmd_status(ctx, pos, flags):
    d = ctx.d
    lines = ["lighting %s | daemon pid %d up %s | hotkey %s" % (
        D.version(), os.getpid(), ago(time.time() - d.started), "Ctrl+Alt+End" if d.hotkey else "unavailable")]
    web().any_browser(ctx)
    hosts = sorted(d.hosts, key=lambda h: -max(h.focused, h.connected))
    if hosts:
        pick = web().pick_host(ctx, quiet=True)
        for h in hosts:
            info = h.info
            old = info.get("ext") and info.get("ext") != D.version()
            lines.append("browser %s %s | ext %s%s%s" % (
                h.brand, info.get("browserVersion", "?"), info.get("ext", "?"),
                " | in use" if h is pick else "",
                (" | updating extension to " + D.version() if h.brand in web()._reloaded else " | OUTDATED, run: lighting setup") if old else ""))
    else:
        lines.append("browser none connected (extension not loaded or browser closed) -> lighting setup")
    if ctx.target:
        lines.append("target %s %s" % (ctx.target[0], ctx.target[1]))
    cfg = ctx.cfg
    lines.append("pointer %s | cleanup %s | browser pref %s | out %s" % (
        "on" if cfg.get("pointer") else "off", "on" if cfg.get("cleanup") else "off", cfg.get("browser"), D.OUT.as_posix()))
    return "\n".join(lines)


def cmd_config(ctx, pos, flags):
    cfg = D.config()
    if not pos:
        return "\n".join("%s = %s" % (k, json.dumps(v)) for k, v in sorted(cfg.items()))
    key = pos[0]
    if key not in D.CONFIG_DEFAULTS:
        raise Fail("unknown config key '%s'" % key, "one of: " + ", ".join(sorted(D.CONFIG_DEFAULTS)))
    if len(pos) == 1:
        return "%s = %s" % (key, json.dumps(cfg.get(key)))
    raw = " ".join(pos[1:])
    default = D.CONFIG_DEFAULTS[key]
    if isinstance(default, bool):
        val = raw.lower() in ("1", "true", "on", "yes")
    elif isinstance(default, int):
        val = int(raw)
    else:
        val = raw
    cfg[key] = val
    D.save_config(cfg)
    ctx.cfg = cfg
    try:
        web().push_config(ctx)
    except Exception:
        pass
    return "%s = %s" % (key, json.dumps(val))


def cmd_log(ctx, pos, flags):
    n = int(pos[0]) if pos and pos[0].isdigit() else 15
    try:
        rows = D.LOG.read_text("utf-8").splitlines()[-n:]
    except OSError:
        return "log is empty"
    out = []
    for r in rows:
        try:
            e = json.loads(r)
        except ValueError:
            continue
        out.append("%s %s %s %s %dms" % (e["t"][11:], "ok " if e["ok"] else "ERR", e["cmd"], e.get("ref", ""), e["ms"]))
    return "\n".join(out) or "log is empty"


def cmd_version(ctx, pos, flags):
    return D.version()


def cmd_help(ctx, pos, flags):
    from lighting.helptext import HELP
    return HELP


def cmd_ext_reload(ctx, pos, flags):
    from lighting import install
    install.copy_extension()
    deadline = time.time() + 12
    while not any(h.alive for h in ctx.d.hosts) and time.time() < deadline:
        time.sleep(0.1)
    old = [h for h in ctx.d.hosts if h.alive]
    if not old:
        return "extension files copied (no browser connected to reload)"
    for h in old:
        h.post("reload-extension")
    deadline = time.time() + 15
    while time.time() < deadline:
        if any(h.alive and h not in old for h in ctx.d.hosts):
            return "extension reloaded"
        time.sleep(0.1)
    return "reload requested, the extension has not reconnected yet"


def cmd_autoload(ctx, pos, flags):
    from lighting import autoload
    return autoload.cmd_autoload(ctx, pos, flags)


def cmd_selftest(ctx, pos, flags):
    from lighting import selftest
    return selftest.run(ctx, pos, flags)


def cmd_bench(ctx, pos, flags):
    from lighting import selftest
    return selftest.bench(ctx, pos, flags)


def cmd_run(ctx, pos, flags):
    from lighting import routines
    return routines.cmd_run(ctx, pos, flags)


def cmd_routine(ctx, pos, flags):
    from lighting import routines
    return routines.cmd_routine(ctx, pos, flags)


def cmd_routines(ctx, pos, flags):
    from lighting import routines
    return routines.cmd_routines(ctx, pos, flags)


def cmd_record(ctx, pos, flags):
    from lighting import record
    return record.cmd_record(ctx, pos, flags)


SYSTEM = {"do": cmd_do, "ping": cmd_ping, "stop": cmd_stop, "status": cmd_status, "config": cmd_config,
          "log": cmd_log, "version": cmd_version, "help": cmd_help, "selftest": cmd_selftest, "bench": cmd_bench, "autoload": cmd_autoload, "ext-reload": cmd_ext_reload,
          "run": cmd_run, "routine": cmd_routine, "routines": cmd_routines, "record": cmd_record,
          "done": cmd_done, "keep": cmd_keep}
READS = {"snap", "text", "table", "read", "js", "fetch", "tabs", "windows", "shot", "log", "status", "downloads",
         "console", "expect", "wait", "clip", "help", "version", "config", "routines", "frames"}
SHARED = {"snap", "click", "type", "press", "shot", "scroll", "hover", "drag"}
APP_ONLY = {"windows", "focus", "clip", "launch"}
WEB_ONLY = {"open", "text", "fill", "select", "check", "wait", "expect", "table", "fetch", "js", "dismiss",
            "upload", "tabs", "tab", "close", "back", "forward", "reload", "dialog", "downloads", "console", "viewport",
            "frames"}


def cap_lines(text, name, ctx=None):
    return cap(text, name, lines=D.SNAP_LINES)

import json
import os
import shlex
import sys
import time

from lighting import defaults as D
from lighting.common import Fail, Pending, cap, is_url, ref_kind

VALUED = {"f", "s", "d", "max", "browser", "lang", "timeout", "width", "until", "pick", "frame",
          "body", "method", "button", "region", "delta", "file", "role", "last", "on", "count", "every", "from", "to",
          "reload", "since", "seconds", "dir", "name"}
TRACE = (D.HOME / "trace").exists()
ALIAS = {"-f": "f", "-s": "s", "-d": "d", "-n": "new", "-a": "all", "-y": "yes", "-e": "errors", "-g": "gone",
         "-m": "media"}


class Context:
    def __init__(self, daemon, sid=""):
        self.d = daemon
        self.sid = sid
        self.used = time.time()
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
        self.tabs = []
        self.owned = set()
        self.prompts = []
        self.told = {}
        self.ep = None
        self.run_stack = []
        self.quiet_steps = 0
        self.group = "Lighting"

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
        elif key in VALUED and i + 1 < len(argv) and not argv[i + 1].startswith("--"):
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
    if verb == "press" and pos and not on:
        from lighting import win
        if all(p.lower() in win.MEDIA for p in pos):
            return app()
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


def app_wait(ctx, pos, flags):
    if flags.get("on") in ("app", "web"):
        return flags["on"] == "app"
    if any(ref_kind(p) == "w" or p.lower().startswith("app:") for p in pos):
        return True
    return ctx.kind() == "app" and bool(pos) and not pos[0].isdigit() and not ref_kind(pos[0]) and not pos[0].startswith(("url:", "css:"))


def route(ctx, name, pos, flags):
    if name in SYSTEM:
        return SYSTEM[name](ctx, pos, flags)
    if name == "read":
        if pos and (is_url(pos[0]) or pos[0].lower().endswith((".pdf",) + web().IMAGE_EXT)):
            return web().read_url(ctx, pos, flags)
        return app().cmd_read(ctx, pos, flags)
    if name == "close" and pos and (ref_kind(pos[0]) == "w" or pos[0].lower().startswith("app:")):
        return app().cmd_close(ctx, pos, flags)
    if name in CHAT:
        from lighting import chat
        return getattr(chat, "cmd_" + name)(ctx, pos, flags)
    if name == "listen":
        from lighting import audio
        return audio.cmd_listen(ctx, pos, flags)
    if name == "claude":
        from lighting import spawn
        return spawn.cmd_claude(ctx, pos, flags)
    if name in APP_ONLY:
        return getattr(app(), "cmd_" + name)(ctx, pos, flags)
    if name == "shot" and flags.get("seconds"):
        return app().cmd_video(ctx, pos, flags)
    if name == "wait" and app_wait(ctx, pos, flags):
        return app().cmd_wait(ctx, pos, flags)
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
        return with_view(ctx, name, flags, route(ctx, name, pos, flags)), name, pos
    try:
        t0 = time.perf_counter()
        out = with_view(ctx, name, flags, route(ctx, name, pos, flags))
        t1 = time.perf_counter()
    except Fail as e:
        routines.observe(ctx, name, pos, flags, e.text(), False)
        raise
    routines.observe(ctx, name, pos, flags, out if isinstance(out, str) else "", True)
    if TRACE:
        print("trace %s: route %.2f ms, observe %.2f ms" % (name, (t1 - t0) * 1000, (time.perf_counter() - t1) * 1000),
              file=sys.stderr, flush=True)
    return out, name, pos


POST_VIEW = {"click", "type", "press", "fill", "select", "check", "scroll", "hover", "drag", "do", "run", "search",
             "back", "forward", "reload", "dismiss", "focus"}


def with_view(ctx, name, flags, out):
    if not flags.get("f") or name not in POST_VIEW or not isinstance(out, str):
        return out
    try:
        view = route(ctx, "snap", [], {"f": flags["f"]})
    except Fail as e:
        view = e.text()
    return out + "\n" + view


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
    if isinstance(out, Pending):
        out.meta = (name, argv, t)
        out.gen = ctx.d.abort_gen
        return {"pending": out}
    if isinstance(out, dict):
        return out
    if name == "suggest":
        return {"out": out if code == 0 else "", "code": 0}
    return finish(ctx, name, argv, out, code, t)


def poll_pending(ctx, p):
    name, argv, t = p.meta
    if p.gen != ctx.d.abort_gen:
        return finish(ctx, name, argv, "stopped (Ctrl+Alt+End or lighting abort)", 1, t)
    last = time.time() >= p.deadline
    saved = (ctx.target, ctx.where, ctx.last_target)
    ctx.image = None
    code = 0
    try:
        out = p.poll(last)
    except Fail as e:
        out, code = e.text(), 1
    except TimeoutError as e:
        out, code = "err: " + str(e), 1
    except Exception as e:
        out, code = "err: %s: %s" % (type(e).__name__, str(e)[:300]), 1
    finally:
        ctx.target, ctx.where, ctx.last_target = saved
    if out is None and not last:
        p.due = time.time() + p.interval
        return None
    return finish(ctx, name, argv, out or "", code, t)


def drain(ctx, out):
    while isinstance(out, Pending):
        if ctx.d.abort.is_set():
            raise Fail("stopped by hotkey")
        time.sleep(max(0.0, out.due - time.time()))
        last = time.time() >= out.deadline
        res = out.poll(last)
        if res is None and not last:
            out.due = time.time() + out.interval
            continue
        return res or ""
    return out


def finish(ctx, name, argv, out, code, t):
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
    log(name, argv, code, (time.perf_counter() - t) * 1000, out, ctx.no_learn)
    if TRACE:
        print("trace %s: total %.2f ms, log %.2f ms" % (name, (time.perf_counter() - t) * 1000, (time.perf_counter() - t_log) * 1000),
              file=sys.stderr, flush=True)
    res = {"out": out, "code": code}
    if ctx.image:
        res["image"] = ctx.image
    return res


_log = {"f": None, "n": 0}


def log(name, argv, code, ms, out="", test=False):
    if name in ("ping", "status", "log", "suggest"):
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
        row = {"t": time.strftime("%Y-%m-%d %H:%M:%S"), "cmd": name, "ref": ref, "ok": code == 0, "ms": int(ms),
               "n": len(out or "")}
        if "-f" in argv[1:]:
            row["f"] = 1
        if test:
            row["x"] = 1
        if code:
            row["e"] = (out or "").replace("err: ", "", 1).split(" -> try:")[0][:60]
        _log["f"].write(json.dumps(row, ensure_ascii=False) + "\n")
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
            out = drain(ctx, out)
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
            replies.append(h.call(cmd, args or {}, timeout=5, group=ctx.group))
        except TimeoutError:
            pass
    return replies


def cmd_done(ctx, pos, flags):
    quiet = flags.get("quiet")
    scope = {"only": sorted(ctx.owned)} if ctx.sid else {}
    if not ctx.cfg.get("cleanup"):
        browsers(ctx, "cleanup", dict(scope, close=False))
        return "" if quiet else "left everything open (cleanup is off: lighting config cleanup on)"
    if ctx.sid and not ctx.owned and not app().state(ctx).launched:
        return "" if quiet else "nothing to close"
    replies = browsers(ctx, "cleanup", scope)
    tabs = sum(r.get("closed") or 0 for r in replies)
    viewed = sum(r.get("kept") or 0 for r in replies)
    closed, left = [], []
    for c in [ctx]:
        got = app().cleanup(c)
        closed += got[0]
        left += got[1]
    if ctx.sid:
        gone = {t for r in replies for t in r.get("shut") or []}
        ctx.tabs = [t for t in ctx.tabs if t not in gone]
        ctx.owned = set()
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
        args = {"id": spec, "only": sorted(ctx.owned)} if ctx.sid and not spec else {"id": spec}
        tabs = sum(r.get("kept") or 0 for r in browsers(ctx, "keep", args))
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
    onoff = lambda k: "on" if cfg.get(k, True) else "off"
    lines.append("pointer %s | cleanup %s | own window %s | browser pref %s | out %s" % (
        onoff("pointer"), onoff("cleanup"), onoff("window"), cfg.get("browser"), D.OUT.as_posix()))
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


def log_stats(rows):
    import collections
    import datetime
    seen = collections.OrderedDict()
    prev = None
    for e in rows:
        s = seen.setdefault(e["cmd"], {"runs": 0, "err": 0, "chars": 0, "sized": 0, "ms": 0, "snap_after": 0})
        s["runs"] += 1
        s["err"] += 0 if e.get("ok") else 1
        s["ms"] += e.get("ms", 0)
        if "n" in e:
            s["chars"] += e["n"]
            s["sized"] += 1
        if prev and e["cmd"] == "snap" and prev["cmd"] in ACTION_VERBS and not prev.get("f"):
            gap = (datetime.datetime.fromisoformat(e["t"]) - datetime.datetime.fromisoformat(prev["t"])).total_seconds()
            if gap <= 15:
                seen[prev["cmd"]]["snap_after"] += 1
        prev = e
    out = ["cmd          runs  err%  avg ms  avg tok  snap after"]
    for cmd, s in sorted(seen.items(), key=lambda kv: -kv[1]["runs"]):
        tok = "%7d" % (s["chars"] // s["sized"] // 4 + 1) if s["sized"] else "      -"
        after = "%d%%" % (100 * s["snap_after"] // s["runs"]) if cmd in ACTION_VERBS else ""
        out.append("%-12s %5d %4d%% %7d  %s  %s" % (cmd, s["runs"], 100 * s["err"] // s["runs"], s["ms"] // s["runs"], tok, after))
    return "\n".join(out)


ACTION_VERBS = {"click", "type", "press", "fill", "select", "check", "scroll", "hover", "open", "launch", "do", "run"}


def cmd_log(ctx, pos, flags):
    if pos and pos[0].lower() == "stats":
        pos = pos[1:]
        flags = dict(flags, stats=True)
    n = int(pos[0]) if pos and pos[0].isdigit() else 15
    if flags.get("stats"):
        rows = []
        try:
            for r in D.LOG.read_text("utf-8").splitlines():
                try:
                    rows.append(json.loads(r))
                except ValueError:
                    pass
        except OSError:
            return "log is empty"
        rows = [r for r in rows if not r.get("x") and r.get("cmd") not in ("selftest", "bench", "done", "ext-reload")]
        rows = rows[-(int(pos[0]) if pos and pos[0].isdigit() else 2000):]
        return cap(log_stats(rows), "log-stats", lines=40) if rows else "log is empty"
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


def cmd_keys(ctx, pos, flags):
    from lighting import keys
    return keys.cmd_keys(ctx, pos, flags)


def cmd_suggest(ctx, pos, flags):
    from lighting import recall
    return recall.cmd_suggest(ctx, pos, flags)


SYSTEM = {"do": cmd_do, "ping": cmd_ping, "stop": cmd_stop, "status": cmd_status, "config": cmd_config,
          "log": cmd_log, "version": cmd_version, "help": cmd_help, "selftest": cmd_selftest, "bench": cmd_bench, "autoload": cmd_autoload, "ext-reload": cmd_ext_reload,
          "run": cmd_run, "routine": cmd_routine, "routines": cmd_routines, "record": cmd_record,
          "done": cmd_done, "keep": cmd_keep, "keys": cmd_keys, "suggest": cmd_suggest}
READS = {"snap", "text", "table", "read", "js", "fetch", "tabs", "windows", "shot", "log", "status", "downloads",
         "console", "expect", "wait", "clip", "help", "version", "config", "routines", "frames", "inbox", "keys",
         "listen", "unread", "net", "tools"}
SHARED = {"snap", "click", "type", "press", "shot", "scroll", "hover", "drag"}
APP_ONLY = {"windows", "focus", "clip", "launch"}
CHAT = {"inbox", "reply", "unread"}
WEB_ONLY = {"open", "text", "fill", "select", "check", "wait", "expect", "table", "fetch", "js", "dismiss", "search",
            "net", "tools", "call",
            "upload", "tabs", "tab", "close", "back", "forward", "reload", "dialog", "downloads", "console", "viewport",
            "frames"}


def cap_lines(text, name, ctx=None):
    return cap(text, name, lines=D.SNAP_LINES)

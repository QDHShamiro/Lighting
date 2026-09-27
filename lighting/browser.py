import json
import math
import os
import re
import subprocess
import time

from lighting import defaults as D
from lighting.common import Fail, Pending, cap, hit, is_url, outfile, parse_ms, plain_links, ref_kind, terms, win_path

REF_RE = re.compile(r"^(?:f\d+\.)?e\d+$")
IMAGE_EXT = (".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp")
_reloaded = set()


def any_browser(ctx):
    end = ctx.d.started + D.RECONNECT_GRACE
    while not any(h.alive for h in ctx.d.hosts):
        if time.time() >= end:
            return False
        time.sleep(0.05)
    return True


def blocklist():
    try:
        rows = D.BLOCKLIST.read_text("utf-8").splitlines()
    except OSError:
        return []
    return [r.strip().lower().lstrip(".") for r in rows if r.strip() and not r.strip().startswith("#")]


def config_payload(ctx_cfg):
    return {"blocklist": blocklist(), "risk": D.RISK_WORDS, "pointer": bool(ctx_cfg.get("pointer", True)),
            "navLines": D.NAV_LINES, "navChars": D.NAV_CHARS}


def on_connect(daemon, host):
    host.post("config", config_payload(D.config()))
    ext = host.info.get("ext")
    if ext and ext != D.version() and host.brand not in _reloaded and (D.EXT / "manifest.json").exists():
        _reloaded.add(host.brand)
        host.post("reload-extension")


def push_config(ctx):
    payload = config_payload(ctx.cfg)
    for h in ctx.d.hosts:
        h.post("config", payload)


def pick_host(ctx, quiet=False, want=None):
    hosts = [h for h in ctx.d.hosts if h.alive]
    if not hosts:
        return None
    pref = (want or ctx.cfg.get("browser") or "auto").lower()
    if pref != "auto":
        for h in hosts:
            if pref in h.brand:
                return h
        if not quiet:
            raise Fail("%s is not connected (connected: %s)" % (pref, ", ".join(h.brand for h in hosts)),
                       "lighting config browser auto")
    return max(hosts, key=lambda h: (h.focused, h.connected))


def running(exe_name):
    try:
        out = subprocess.run(["tasklist", "/FI", "IMAGENAME eq " + exe_name, "/NH"], capture_output=True,
                             text=True, timeout=5, creationflags=0x08000000).stdout
    except (OSError, subprocess.SubprocessError):
        return False
    return exe_name.lower() in out.lower()


def ensure_host(ctx, want=None):
    host = pick_host(ctx, want=want)
    if host:
        return host
    from lighting import install
    pref = (want or ctx.cfg.get("browser") or "auto").lower()
    order = [pref] if pref in D.BROWSER_EXES else D.BROWSER_ORDER
    for name in order:
        exe = install.browser_exe(name)
        if not exe:
            continue
        if running(os.path.basename(exe)):
            deadline = time.time() + 6
            while time.time() < deadline:
                host = pick_host(ctx, quiet=True, want=want)
                if host:
                    return host
                time.sleep(0.1)
            raise Fail("%s is open but the Lighting extension is not connected" % name,
                       "lighting setup (loads the extension), then lighting status")
        subprocess.Popen([exe], creationflags=0x00000008 | 0x00000200, close_fds=True)
        deadline = time.time() + D.BROWSER_START_WAIT
        while time.time() < deadline:
            host = pick_host(ctx, quiet=True, want=want)
            if host:
                return host
            time.sleep(0.2)
        raise Fail("started %s but the Lighting extension did not connect" % name, "lighting setup")
    raise Fail("no browser with the Lighting extension is connected", "lighting setup")


NEW_TAB = re.compile(r"new tab (t\d+) opened from (t\d+)")
SESSION_FREE = {"config", "abort", "reload-extension", "ping", "close-url", "tabs", "downloads", "cleanup", "keep",
                "tab", "record-start", "record-stop", "record-status"}


def session_tab(ctx, cmd, args):
    if not getattr(ctx, "sid", "") or cmd in SESSION_FREE or (cmd == "close" and args.get("id")):
        return None
    if cmd == "open":
        if args.get("new"):
            return None
        if not ctx.tabs:
            args["new"] = True
        return ctx.tabs[0] if ctx.tabs else None
    if not ctx.tabs:
        raise Fail("this session has no tab yet", "lighting open <url>, or lighting tabs + lighting tab t3 to use one")
    return ctx.tabs[0]


def track(ctx, cmd, msg):
    if not getattr(ctx, "sid", ""):
        return
    t = (msg.get("where") or {}).get("tab")
    if cmd == "open" and msg.get("created") and t:
        ctx.owned.add(t)
    if t and (cmd in ("open", "tab") or t in ctx.owned or (ctx.tabs and ctx.tabs[0] == t)):
        ctx.tabs = [t] + [x for x in ctx.tabs if x != t]
    for ev in list(ctx.d.events):
        m = NEW_TAB.search(ev)
        if m and m.group(2) in ctx.owned and m.group(1) not in ctx.owned:
            ctx.owned.add(m.group(1))
            ctx.tabs = [m.group(1)] + [x for x in ctx.tabs if x != m.group(1)]


def call(ctx, cmd, args=None, timeout=D.CALL_TIMEOUT, want=None, tab=None):
    host = ensure_host(ctx, want)
    args = args if args is not None else {}
    own = session_tab(ctx, cmd, args) if tab is None else tab
    msg = host.call(cmd, args, tab=own, timeout=timeout)
    ctx.target = ("web", host.brand)
    if not msg.get("ok"):
        err = str(msg.get("error") or "browser error")
        if tab is None and own and "no tab yet" in err:
            ctx.tabs = [x for x in ctx.tabs if x != own]
            ctx.owned.discard(own)
            return call(ctx, cmd, args, timeout, want)
        raise Fail(err)
    track(ctx, cmd, msg)
    if msg.get("target"):
        ctx.last_target = msg["target"]
    if msg.get("where"):
        ctx.where = dict(msg["where"], kind="web")
    if msg.get("keys"):
        from lighting import keys
        keys.harvest(keys.app_of(ctx), msg["keys"])
    return msg


def out(ctx, cmd, args=None, timeout=D.CALL_TIMEOUT, name=None, lines=D.SNAP_LINES):
    msg = call(ctx, cmd, args, timeout)
    return cap(msg.get("out", ""), name or cmd, lines=lines)


def target_args(pos, flags=None):
    if not pos:
        raise Fail("need a ref (e12) or \"text\"")
    if REF_RE.match(pos[0]):
        return {"ref": pos[0]}, pos[1:]
    args = {"text": pos[0]}
    if flags and flags.get("first"):
        args["first"] = True
    if flags and flags.get("role"):
        args["role"] = flags["role"]
    return args, pos[1:]


def normalize_url(u):
    if re.match(r"^[\w.-]+:\d+(?:[/?#]|$)", u):
        return "http://" + u
    if re.match(r"^[a-zA-Z][a-zA-Z0-9+.-]*:", u):
        return u
    if re.match(r"^([a-zA-Z]:[\\/]|\\\\|/[a-zA-Z]/)", u):
        return "file:///" + win_path(u).replace("\\", "/")
    return "https://" + u


def leak_check(ctx, url, flags):
    from urllib.parse import urlsplit
    parts = urlsplit(url)
    host = (parts.hostname or "").lower()
    if not host:
        return
    size = len(parts.query) + len(parts.fragment)
    if size > D.LEAK_QUERY and host not in ctx.d.hosts_seen and not flags.get("yes"):
        raise Fail("%d chars of query data to %s, a site not opened in this session (possible data leak)" % (size, host),
                   "check the URL, then rerun with --yes")
    ctx.d.hosts_seen.add(host)


def cmd_open(ctx, pos, flags):
    if not pos:
        raise Fail("open needs a url", "lighting open github.com")
    url = normalize_url(pos[0])
    if url.split(":", 1)[0].lower() not in D.WEB_SCHEMES:
        from lighting import desktop
        return desktop.cmd_launch(ctx, [url], flags)
    leak_check(ctx, url, flags)
    if flags.get("text"):
        head = out(ctx, "open", {"url": url, "new": bool(flags.get("new")), "quiet": True}, D.LOAD_TIMEOUT + 10)
        return head.split("\n", 1)[0] + "\n" + cmd_text(ctx, [], flags)
    msg = call(ctx, "open", {"url": url, "new": bool(flags.get("new")), "filter": flags.get("f"), "scope": flags.get("s"),
                             "media": bool(flags.get("media"))}, D.LOAD_TIMEOUT + 10)
    res = cap(msg.get("out", ""), "open", lines=D.SNAP_LINES)
    from lighting import keys
    extra = keys.hint(keys.app_of(ctx))
    if msg.get("net"):
        extra += "\ndata: %d JSON call%s (lighting net)" % (msg["net"], "" if msg["net"] == 1 else "s")
    if msg.get("webmcp"):
        extra += "\nwebmcp: %d tool%s (lighting tools)" % (msg["webmcp"], "" if msg["webmcp"] == 1 else "s")
    head, nl, rest = res.partition("\n")
    return head + extra + nl + rest


def shape(data, depth=0):
    if isinstance(data, list):
        return "[] (%d)%s" % (len(data), ": " + shape(data[0], depth + 1) if data and depth < 2 else "")
    if isinstance(data, dict):
        parts = []
        for k, v in list(data.items())[:12]:
            if isinstance(v, list):
                parts.append("%s[] (%d)%s" % (k, len(v), " {%s}" % shape(v[0], depth + 1) if v and isinstance(v[0], dict) and depth < 2 else ""))
            elif isinstance(v, dict):
                parts.append("%s{%s}" % (k, ", ".join(list(v)[:6])) if depth >= 1 else "%s{%s}" % (k, shape(v, depth + 1)))
            else:
                s = json.dumps(v, ensure_ascii=False)
                parts.append("%s: %s" % (k, s if len(s) <= 40 else s[:37] + "..."))
        more = len(data) - 12
        return ", ".join(parts) + (", +%d more" % more if more > 0 else "")
    s = json.dumps(data, ensure_ascii=False)
    return s if len(s) <= 60 else s[:57] + "..."


def cmd_net(ctx, pos, flags):
    calls = call(ctx, "net", {"op": "list"}).get("calls") or []
    if not calls:
        return "no JSON answers seen in this tab yet (only GET calls after Lighting attached; reload to capture)"
    rows = list(reversed(calls))
    if pos and pos[0].lstrip("n").isdigit():
        n = int(pos[0].lstrip("n"))
        if not 1 <= n <= len(rows):
            raise Fail("no call n%d" % n, "lighting net")
        got = call(ctx, "net", {"op": "body", "id": rows[n - 1]["id"]})
        try:
            data = json.loads(got.get("body") or "")
        except ValueError:
            return "n%d %s: not JSON" % (n, got.get("url"))
        return "n%d GET %s (%s)\n%s\n-> lighting fetch \"%s\" --pick <path>" % (
            n, short_url(got.get("url")), kb(got.get("size")), shape(data), got.get("url"))
    words = terms(flags.get("f"))
    lines = ["n%d GET %s (json %s)" % (i, short_url(c["url"]), kb(c["size"])) for i, c in enumerate(rows, 1)
             if not words or hit(words, c["url"])]
    return "\n".join(lines or ["no call matches"]) + "\n-> lighting net <n> shows the data"


def kb(size):
    size = int(size or 0)
    return "%d B" % size if size < 1024 else "%.0f KB" % (size / 1024.0)


def short_url(url):
    u = re.sub(r"^https?://(www\.)?", "", url or "")
    return u if len(u) <= 110 else u[:107] + "..."


def cmd_tools(ctx, pos, flags):
    got = call(ctx, "webmcp", {"op": "list"})
    tools = got.get("tools") or []
    if not tools:
        return "this page offers no WebMCP tools%s" % ("" if got.get("api") else " (the browser has no WebMCP here)")
    lines = []
    for t in tools:
        args = ", ".join(sorted(((t.get("schema") or {}).get("properties") or {}).keys()))
        lines.append("%s(%s)%s - %s" % (t["name"], args, " read-only" if t.get("readOnly") else "", t.get("description") or ""))
    return "\n".join(lines) + "\n-> lighting call <tool> '{\"arg\": 1}'"


def cmd_call(ctx, pos, flags):
    if not pos:
        raise Fail("call needs a tool name", "lighting tools")
    try:
        args = json.loads(" ".join(pos[1:]) or "{}")
    except ValueError:
        raise Fail("the arguments must be JSON", "lighting call %s '{\"q\": \"x\"}'" % pos[0])
    tools = {t["name"]: t for t in call(ctx, "webmcp", {"op": "list"}).get("tools") or []}
    tool = tools.get(pos[0])
    if not tool:
        raise Fail("no tool %s on this page" % pos[0], "lighting tools")
    if not tool.get("readOnly") and not flags.get("yes"):
        raise Fail("%s can change things on the site" % pos[0], "only if the user agreed: lighting call %s ... --yes" % pos[0])
    res = call(ctx, "webmcp", {"op": "call", "name": pos[0], "args": args}, 60).get("result")
    return cap("ok %s -> %s" % (pos[0], res), "call", chars=D.OUT_CHARS)


def cmd_snap(ctx, pos, flags):
    args = {"all": bool(flags.get("all")), "diff": bool(flags.get("diff")), "force": bool(flags.get("force")),
            "media": bool(flags.get("media"))}
    if flags.get("f"):
        args["filter"] = flags["f"]
    scope = flags.get("s") or (pos[0] if pos and REF_RE.match(pos[0]) else None)
    if scope:
        args["scope"] = scope
    if flags.get("frame"):
        args["frame"] = flags["frame"]
    limit = None if flags.get("all") else D.SNAP_LINES
    msg = call(ctx, "snap", args, 30)
    return cap(msg.get("out", ""), "snap", lines=limit or 100000)


def cmd_text(ctx, pos, flags):
    msg = call(ctx, "text", {"filter": flags.get("f"), "raw": bool(flags.get("raw")), "links": bool(flags.get("links"))}, 40)
    limit = int(flags.get("max") or D.TEXT_CHARS)
    return cap(msg.get("out", ""), "text", chars=limit)


def cmd_click(ctx, pos, flags):
    args, _ = target_args(pos, flags)
    args.update({"yes": bool(flags.get("yes")), "force": bool(flags.get("force")), "trace": bool(flags.get("trace"))})
    if flags.get("right"):
        args["button"] = "right"
    if flags.get("double"):
        args["count"] = 2
    return out(ctx, "click", args, D.LOAD_TIMEOUT + 10, lines=D.NAV_LINES + 8)


def cmd_type(ctx, pos, flags):
    args, rest = target_args(pos, flags)
    if ctx.secret is not None:
        args.update({"value": ctx.secret, "secret": True})
    else:
        if not rest:
            raise Fail("type needs text", 'lighting type e3 "hello"')
        args["value"] = " ".join(rest)
    args.update({"append": bool(flags.get("append")), "enter": bool(flags.get("enter"))})
    res = out(ctx, "type", args, D.LOAD_TIMEOUT + 10, lines=D.NAV_LINES + 8)
    t = ctx.last_target or {}
    searchy = t.get("role") in ("searchbox", "combobox") or re.search(r"such|search", t.get("name") or "", re.I)
    if flags.get("enter") and not flags.get("nolearn") and ctx.secret is None and searchy:
        from lighting import sites
        sites.learn((ctx.where or {}).get("url"), args.get("value"))
    return res


def cmd_search(ctx, pos, flags):
    from lighting import sites
    if not pos:
        raise Fail("search needs words", 'lighting search "paper plugin" | search youtube "lofi"')
    site = sites.named(pos[0]) if len(pos) > 1 else None
    words = " ".join(pos[1:] if site else pos)
    host = site or sites.host_of((ctx.where or {}).get("url"))
    tpl = sites.template(host) if host else None
    view = {k: v for k, v in flags.items() if k in ("f", "new", "text")}
    if tpl:
        return cmd_open(ctx, [sites.build(tpl, words)], view)
    if site:
        cmd_open(ctx, ["https://" + site], {"new": flags.get("new")})
    found = call(ctx, "search-field", {})
    if found.get("button"):
        call(ctx, "click", {"ref": found["button"]}, D.LOAD_TIMEOUT + 10)
        found = call(ctx, "search-field", {})
    if not found.get("ref"):
        raise Fail("no search field on this page", "lighting snap -f search, then type e<N> words --enter")
    res = cmd_type(ctx, [found["ref"], words], dict(flags, enter=True, nolearn=True))
    learned = sites.learn((ctx.where or {}).get("url"), words)
    return res + ("\n! learned the search URL of %s: next time search jumps there directly" % sites.host_of(learned)
                  if learned else "")


def cmd_fill(ctx, pos, flags):
    pairs = []
    for p in pos:
        label, sep, value = p.partition("=")
        if not sep:
            raise Fail("fill wants Label=value pairs", 'lighting fill "Email=me@x.de" "Password=@secret" --env PASS')
        if value == "@secret":
            if ctx.secret is None:
                raise Fail("@secret needs --env VAR")
            value = ctx.secret
        pairs.append({"label": label.strip(), "value": value})
    if not pairs:
        raise Fail("fill wants Label=value pairs")
    args = {"pairs": pairs, "submit": bool(flags.get("submit")), "yes": bool(flags.get("yes"))}
    return out(ctx, "fill", args, D.LOAD_TIMEOUT + 15, lines=D.NAV_LINES + 8)


def cmd_press(ctx, pos, flags):
    if not pos:
        raise Fail("press needs keys", "lighting press Enter | ctrl+a | Escape")
    res = out(ctx, "press", {"keys": pos}, D.LOAD_TIMEOUT + 10, lines=D.NAV_LINES + 8)
    if len(pos) == 1 and " new" in res.split("\n", 1)[0] and "\n" in res:
        from lighting import keys
        keys.learn(keys.app_of(ctx), pos[0], res.split("\n", 2)[1])
    return res


def cmd_select(ctx, pos, flags):
    args, rest = target_args(pos, flags)
    if not rest:
        raise Fail("select needs an option", 'lighting select e7 "Germany"')
    args["option"] = " ".join(rest)
    return out(ctx, "select", args, D.LOAD_TIMEOUT)


def cmd_check(ctx, pos, flags):
    args, rest = target_args(pos, flags)
    args["on"] = not (rest and rest[0].lower() in ("off", "false", "0", "no", "uncheck"))
    return out(ctx, "check", args, D.LOAD_TIMEOUT)


def cmd_hover(ctx, pos, flags):
    args, _ = target_args(pos, flags)
    return out(ctx, "hover", args, D.LOAD_TIMEOUT)


def cmd_drag(ctx, pos, flags):
    if len(pos) < 2:
        raise Fail("drag needs two refs", "lighting drag e3 e9")
    return out(ctx, "drag", {"from": pos[0], "to": pos[1]}, D.LOAD_TIMEOUT)


def cmd_scroll(ctx, pos, flags):
    args = {}
    for p in pos:
        if REF_RE.match(p):
            args["ref"] = p
        elif p.lower() in ("up", "down", "top", "bottom"):
            args["dir"] = p.lower()
    if flags.get("until"):
        args["until"] = flags["until"]
    if flags.get("delta"):
        args["delta"] = int(flags["delta"])
    if flags.get("max"):
        args["max"] = int(flags["max"])
    return out(ctx, "scroll", args, 60)


def spec(pos, flags):
    s = {"gone": bool(flags.get("gone"))}
    if not pos:
        raise Fail("needs \"text\", url:part, css:selector, a ref or milliseconds")
    p = " ".join(pos)
    if p.startswith("url:"):
        s["url"] = p[4:]
    elif p.startswith("css:"):
        s["css"] = p[4:]
    elif REF_RE.match(p):
        s["ref"] = p
    elif p.isdigit():
        s["ms"] = int(p)
    else:
        s["text"] = p
    return s


def cmd_wait(ctx, pos, flags):
    s = spec(pos, flags)
    if "ms" in s and not flags.get("reload"):
        time.sleep(min(s["ms"], 60000) / 1000.0)
        return "ok (%d ms)" % min(s["ms"], 60000)
    if flags.get("reload"):
        return wait_reload(ctx, s, flags)
    s["timeout"] = parse_ms(flags.get("timeout"), 10000)
    return out(ctx, "wait", s, s["timeout"] / 1000 + 10)


def wait_reload(ctx, s, flags):
    if "ms" in s:
        raise Fail('--reload waits for "text", url:, css: or a ref', 'lighting wait "All checks have passed" --reload 15s')
    every = max(parse_ms(flags.get("reload"), 15000) / 1000.0, 3.0)
    total = parse_ms(flags.get("timeout"), 600000) / 1000.0
    tab = (call(ctx, "where").get("where") or {}).get("tab")
    t0 = time.time()
    st = {"reloads": 0, "next": t0 + every}

    def found():
        try:
            call(ctx, "expect", s, tab=tab)
            return True
        except Fail as e:
            if str(e).startswith("fail:"):
                return False
            raise

    def poll(last):
        if found():
            return "ok after %d reload%s (%d s)" % (st["reloads"], "" if st["reloads"] == 1 else "s", time.time() - t0)
        if last:
            raise Fail("not there after %d s (%d reloads)" % (total, st["reloads"]),
                       "lighting wait ... --reload %ds --timeout %dm" % (every, total // 30 or 1))
        if time.time() >= st["next"]:
            call(ctx, "reload", {}, D.LOAD_TIMEOUT + 10, tab=tab)
            st["reloads"] += 1
            st["next"] = time.time() + every
        return None

    if found():
        return "ok (already there)"
    return Pending(poll, total, 1.0)


def cmd_expect(ctx, pos, flags):
    return out(ctx, "expect", spec(pos, flags), 20)


def cmd_table(ctx, pos, flags):
    return out(ctx, "table", {"target": pos[0] if pos else None}, 20, lines=120)


def pick(data, path):
    cur = [data]
    for part in re.findall(r"[^.\[\]]+|\[\d*\]", path):
        nxt = []
        for c in cur:
            if part == "[]":
                if isinstance(c, list):
                    nxt.extend(c)
            elif part.startswith("["):
                i = int(part[1:-1])
                if isinstance(c, list) and -len(c) <= i < len(c):
                    nxt.append(c[i])
            elif isinstance(c, dict) and part in c:
                nxt.append(c[part])
            elif isinstance(c, list):
                nxt.extend(x[part] for x in c if isinstance(x, dict) and part in x)
        cur = nxt
    return cur[0] if len(cur) == 1 and "[]" not in path else cur


def cmd_fetch(ctx, pos, flags):
    if not pos:
        raise Fail("fetch needs a url", "lighting fetch /api/user --pick login")
    leak_check(ctx, pos[0], flags)
    args = {"url": pos[0], "method": (flags.get("method") or "GET").upper(), "cookies": bool(flags.get("cookies"))}
    if flags.get("body"):
        args["body"] = flags["body"]
    res = call(ctx, "fetch", args, 30).get("fetch") or {}
    body = res.get("text", "")
    head = "%s %s" % (res.get("status"), (res.get("type") or "").split(";")[0])
    try:
        data = json.loads(body)
    except ValueError:
        if "html" in (res.get("type") or ""):
            from lighting import html2md
            title, body = html2md.convert(body, res.get("url") or pos[0])
            head += " (html converted)"
        return cap(head + "\n" + body, "fetch", chars=int(flags.get("max") or D.TEXT_CHARS))
    if flags.get("pick"):
        data = pick(data, flags["pick"])
    text = json.dumps(data, ensure_ascii=False, separators=(",", ":"))
    return cap(head + "\n" + text, "fetch", chars=D.TEXT_CHARS)


def cmd_js(ctx, pos, flags):
    code = " ".join(pos)
    if flags.get("file"):
        path = win_path(flags["file"])
        if not os.path.isabs(path):
            path = os.path.join(ctx.cwd or os.getcwd(), path)
        try:
            with open(path, encoding="utf-8") as f:
                code = f.read()
        except OSError as e:
            raise Fail("cannot read %s: %s" % (path, e.strerror))
    if not code.strip():
        raise Fail("js needs code", "lighting js \"document.title\" or lighting js --file script.js")
    return out(ctx, "js", {"code": code}, 25, lines=200)


def cmd_dismiss(ctx, pos, flags):
    return out(ctx, "dismiss", {}, 20)


def cmd_upload(ctx, pos, flags):
    if len(pos) < 2 or not REF_RE.match(pos[0]):
        raise Fail("upload needs a ref and files", "lighting upload e5 C:/path/file.png")
    files = []
    for f in pos[1:]:
        p = win_path(f)
        if not os.path.isabs(p):
            p = os.path.join(ctx.cwd or os.getcwd(), p)
        p = os.path.normpath(p)
        if not os.path.exists(p):
            raise Fail("file not found: " + p)
        files.append(p)
    return out(ctx, "upload", {"ref": pos[0], "files": files}, 30)


def cmd_tabs(ctx, pos, flags):
    return out(ctx, "tabs", {}, 15, lines=80)


def cmd_tab(ctx, pos, flags):
    if not pos:
        raise Fail("tab needs an id", "lighting tabs")
    return out(ctx, "tab", {"id": pos[0]}, 15)


def cmd_close(ctx, pos, flags):
    res = out(ctx, "close", {"id": pos[0] if pos else None, "force": bool(flags.get("force"))}, 15)
    m = re.match(r"closed (t\d+)", res)
    if m and getattr(ctx, "sid", ""):
        gone = m.group(1)
        ctx.tabs = [x for x in ctx.tabs if x != gone]
        ctx.owned.discard(gone)
        res = "closed %s, %s" % (gone, "target now " + ctx.tabs[0] if ctx.tabs else "no tab left in this session")
    return res


def cmd_back(ctx, pos, flags):
    return out(ctx, "back", {}, D.LOAD_TIMEOUT + 10)


def cmd_forward(ctx, pos, flags):
    return out(ctx, "forward", {}, D.LOAD_TIMEOUT + 10)


def cmd_reload(ctx, pos, flags):
    return out(ctx, "reload", {}, D.LOAD_TIMEOUT + 10)


def cmd_dialog(ctx, pos, flags):
    if not pos or pos[0].lower() not in ("accept", "dismiss", "ok", "cancel"):
        raise Fail("dialog accept|dismiss [text]")
    accept = pos[0].lower() in ("accept", "ok")
    text = " ".join(pos[1:]) if len(pos) > 1 else None
    return out(ctx, "dialog", {"accept": accept, "text": text}, 15)


def cmd_downloads(ctx, pos, flags):
    return out(ctx, "downloads", {}, 15)


def cmd_console(ctx, pos, flags):
    return out(ctx, "console", {}, 15, lines=60)


def cmd_shot(ctx, pos, flags):
    from PIL import Image
    import base64
    import hashlib
    import io
    ref = pos[0] if pos and REF_RE.match(pos[0]) else None
    msg = call(ctx, "shot", {"ref": ref, "marks": bool(flags.get("marks"))}, 30)
    img = Image.open(io.BytesIO(base64.b64decode(msg["image"])))
    crop = msg.get("crop")
    if crop and msg.get("vw"):
        s = img.width / float(msg["vw"])
        box = (max(0, int(crop["x"] * s)), max(0, int(crop["y"] * s)),
               min(img.width, int((crop["x"] + crop["w"]) * s)), min(img.height, int((crop["y"] + crop["h"]) * s)))
        if box[2] > box[0] and box[3] > box[1]:
            img = img.crop(box)
    res = save_image(ctx, img, "web-" + (ref or "view"), bool(flags.get("if-changed")),
                     int(flags.get("width") or ctx.cfg.get("shot_width") or D.SHOT_WIDTH))
    if flags.get("marks"):
        res += "\n%d orange labels = e-refs (label 12 -> lighting click e12)" % msg.get("marks", 0)
    return res


def seconds(text):
    if text in (None, "", True):
        return None
    total = 0.0
    for part in str(text).split(":"):
        total = total * 60 + float(part)
    return total


def clock(s):
    s = int(max(0, s))
    return "%d:%02d:%02d" % (s // 3600, s // 60 % 60, s % 60) if s >= 3600 else "%d:%02d" % (s // 60, s % 60)


def distinct(tiles, n):
    from PIL import ImageChops, ImageStat
    thumbs = [t.convert("L").resize((32, 18)) for t in tiles]
    gap = lambda a, b: ImageStat.Stat(ImageChops.difference(thumbs[a], thumbs[b])).mean[0]
    chosen = [0]
    while len(chosen) < min(n, len(tiles)):
        chosen.append(max((i for i in range(len(tiles)) if i not in chosen), key=lambda i: min(gap(i, j) for j in chosen)))
    return sorted(chosen)


def cmd_frames(ctx, pos, flags):
    from PIL import Image, ImageDraw
    import base64
    import hashlib
    import io
    if not pos or not REF_RE.match(pos[0]):
        raise Fail("frames needs the ref of a video", "lighting snap -f video, then lighting frames e40")
    count = max(2, min(int(flags.get("count") or 6), 12))
    every = max(200, min(int(flags.get("every") or 1500), 10000))
    scenes, live = bool(flags.get("scenes")), bool(flags.get("live"))
    try:
        lo, hi = seconds(flags.get("from")), seconds(flags.get("to"))
    except ValueError:
        raise Fail("--from and --to take seconds or m:ss", "lighting frames e40 --from 1:30 --to 2:00")
    samples = count * 4 if scenes else count
    msg = call(ctx, "frames", {"ref": pos[0], "count": count, "every": every, "live": live, "from": lo, "to": hi,
                               "samples": samples}, 30 + (count * every // 1000 if live else samples * 8))
    frames = msg.get("frames") or []
    if not frames:
        raise Fail("no frames captured", "lighting shot " + pos[0])
    tiles = [Image.open(io.BytesIO(base64.b64decode(f["image"]))).convert("RGB") for f in frames]
    if scenes and not msg.get("live"):
        keep = distinct(tiles, count)
        tiles, frames = [tiles[i] for i in keep], [frames[i] for i in keep]
    stamps = [(f.get("media") or "").split("/")[0].split(" ")[0] or "?" for f in frames]
    cols = 3 if len(tiles) > 4 else 2
    tw = 400
    th = max(1, round(tiles[0].height * tw / tiles[0].width))
    sheet = Image.new("RGB", (cols * tw, -(-len(tiles) // cols) * th), "black")
    draw = ImageDraw.Draw(sheet)
    for i, (tile, stamp) in enumerate(zip(tiles, stamps)):
        x, y = i % cols * tw, i // cols * th
        sheet.paste(tile.resize((tw, th)), (x, y))
        draw.rectangle((x, y, x + 8 + 7 * len(stamp), y + 16), fill="black")
        draw.text((x + 4, y + 2), stamp, fill="white")
    res = save_image(ctx, sheet, "frames-" + pos[0], False, int(flags.get("width") or ctx.cfg.get("shot_width") or D.SHOT_WIDTH))
    looks = {hashlib.md5(t.convert("L").resize((32, 18)).point(lambda v: v // 24 * 24).tobytes()).hexdigest() for t in tiles}
    still = len(looks) == 1 or (len(set(stamps)) == 1 and ":" in stamps[0])
    note = ""
    if still:
        note = (" | the video did not move (paused or not playing) -> try: click %s or press k, then frames again" % pos[0]
                if msg.get("live") else " | all frames look the same (black or not decoding) -> try: lighting frames %s --live" % pos[0])
    if msg.get("live"):
        mode = "live, every %.1f s" % (every / 1000.0) + ("" if live else ", video not seekable")
    else:
        span = "%s-%s of %s" % (clock(lo or 0), clock(hi or msg.get("duration") or 0), clock(msg.get("duration") or 0))
        mode = ("%d distinct scenes from %d samples, " % (len(tiles), samples) if scenes else "spread over ") + span
    lines = [res.replace("shot ", "frames ", 1), "%d frames (%s) at %s%s" % (len(tiles), mode, " ".join(stamps), note)]
    caps = ["%s %s" % (s, f["cap"][:140]) for s, f in zip(stamps, frames) if f.get("cap")]
    if caps:
        lines += ["captions:"] + caps
    if flags.get("audio"):
        from lighting import audio
        from lighting.commands import drain
        try:
            heard = drain(ctx, audio.cmd_listen(ctx, [pos[0]], {k: flags.get(k) for k in ("from", "to", "lang", "local")}))
            lines += heard.split("\n", 1)[1:]
        except Fail as e:
            lines.append("audio: " + e.text())
    return "\n".join(lines)


def image_note(ctx, raw):
    from PIL import Image
    import io
    try:
        img = Image.open(io.BytesIO(raw))
        img.load()
    except Exception as e:
        raise Fail("not a readable image (%s)" % str(e)[:80])
    return save_image(ctx, img, "image", False, int(ctx.cfg.get("shot_width") or D.SHOT_WIDTH))


def save_image(ctx, img, key, if_changed, width):
    from PIL import Image
    import hashlib
    if img.width > width:
        img = img.resize((width, max(1, round(img.height * width / img.width))), Image.LANCZOS)
    thumb = img.convert("L").resize((48, 27)).point(lambda v: v // 24 * 24)
    digest = hashlib.md5(thumb.tobytes()).hexdigest()
    if if_changed and ctx.last_shot.get(key) == digest:
        return "unchanged since the last shot of %s (no new image)" % key
    ctx.last_shot[key] = digest
    path = outfile("shot-" + key, "jpg")
    img.convert("RGB").save(path, "JPEG", quality=D.JPEG_QUALITY)
    ctx.image = str(path)
    cost = math.ceil(img.width / 28) * math.ceil(img.height / 28)
    return "shot %s (%dx%d, ~%d image tokens; open it with the Read tool)" % (path.as_posix(), img.width, img.height, cost)


def pdf_text(raw):
    import io
    from pypdf import PdfReader
    pages = PdfReader(io.BytesIO(raw)).pages
    return "\n\n".join("[p%d] %s" % (i, (p.extract_text() or "").strip()) for i, p in enumerate(pages, 1)), len(pages)


_tls = []


def tls():
    if not _tls:
        import ssl
        try:
            import truststore
            _tls.append(truststore.SSLContext(ssl.PROTOCOL_TLS_CLIENT))
        except ImportError:
            _tls.append(ssl.create_default_context())
    return _tls[0]


def fetch_page(ctx, url):
    import urllib.request
    from lighting import html2md
    if not is_url(url):
        path = win_path(url)
        if not os.path.isabs(path):
            path = os.path.join(ctx.cwd or os.getcwd(), path)
        try:
            with open(path, "rb") as f:
                raw = f.read(D.PDF_BYTES)
        except OSError as e:
            raise Fail("cannot read %s: %s" % (path, e.strerror))
        if path.lower().endswith(IMAGE_EXT):
            return "", image_note(ctx, raw), path, "image"
        text, n = pdf_text(raw)
        return "", text, path, "pdf, %d pages" % n
    req = urllib.request.Request(url, headers={
        "Accept": "text/markdown, text/plain;q=0.9, text/html;q=0.8, */*;q=0.1",
        "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) lighting/" + D.version(),
    })
    try:
        with urllib.request.urlopen(req, timeout=20, context=tls()) as r:
            ctype = r.headers.get("content-type", "")
            pdf = "pdf" in ctype or url.lower().split("?")[0].endswith(".pdf")
            raw = r.read(D.PDF_BYTES if pdf else 4_000_000)
            final = r.geturl()
            charset = r.headers.get_content_charset() or "utf-8"
    except Exception as e:
        raise Fail("read failed: %s" % str(e)[:200], "lighting open " + url + " (uses the real browser)")
    if pdf or raw[:5] == b"%PDF-":
        text, n = pdf_text(raw)
        return "", text, final, "pdf, %d pages" % n
    if ctype.startswith("image/"):
        return "", image_note(ctx, raw), final, "image"
    body = raw.decode(charset, errors="replace")
    if "markdown" in ctype or "text/plain" in ctype:
        return "", body.strip(), final, "markdown from server"
    if "html" in ctype:
        title, text = html2md.convert(body, final)
        return title, text, final, "html converted"
    raise Fail("not a text page (%s)" % ctype.split(";")[0], "lighting open " + url)


def read_one(ctx, url, flags, chars):
    title, text, final, how = fetch_page(ctx, url)
    if not flags.get("links"):
        text = plain_links(text)
    if flags.get("f"):
        fl = terms(flags["f"])
        parts = re.split(r"\n\s*\n", text)
        text = "\n\n".join(p for p in parts if hit(fl, p)) or "(no paragraph contains '%s')" % flags["f"]
    head = "[read] %s - %s (%s)" % ((title or "")[:70], final[:90], how)
    return cap(head + "\n" + text, "read", chars=chars)


def read_url(ctx, pos, flags):
    from concurrent.futures import ThreadPoolExecutor
    chars = int(flags.get("max") or D.TEXT_CHARS)
    for url in pos:
        if is_url(url):
            leak_check(ctx, url, flags)
    if len(pos) == 1:
        return read_one(ctx, pos[0], flags, chars)
    each = max(1500, chars // len(pos))

    def one(url):
        try:
            return read_one(ctx, url, flags, each)
        except Fail as e:
            return "[read] %s\n%s" % (url, e.text())

    with ThreadPoolExecutor(max_workers=min(8, len(pos))) as pool:
        return "\n\n".join(pool.map(one, pos))


def cmd_viewport(ctx, pos, flags):
    if not pos:
        raise Fail("viewport needs WxH or reset", "lighting viewport 390x844")
    if pos[0] in ("reset", "off"):
        return out(ctx, "viewport", {})
    m = re.match(r"^(\d{2,4})x(\d{2,4})$", pos[0])
    if not m:
        raise Fail("viewport wants WxH like 390x844", "lighting viewport 390x844")
    return out(ctx, "viewport", {"width": int(m.group(1)), "height": int(m.group(2))})

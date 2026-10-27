import difflib
import json
import re
import time
from urllib.parse import urlsplit

from lighting import defaults as D
from lighting.common import Fail, cap, note, ref_kind, trimmed

LEARN_CMDS = {"click", "type", "select", "check", "hover", "fill"}
TARGETED = {"click", "type", "select", "check", "hover"}
ELEMENT = re.compile(r'^((?:f\d+\.)?[ed]\d+) ([\w-]+) "((?:[^"\\]|\\.)*)"')
HEADER = re.compile(r"^\[(t\d+)")
GONE = re.compile(r"is gone")
MISS = re.compile(r"nothing matches|nothing called|no field called")
ALWAYS = {"alert", "alertdialog", "dialog", "status"}
ADVICE = [
    (r"no tab yet|has no tab", "open a page first (lighting open <url>); a script must stop when its tab is gone"),
    (r"is gone|older snapshot", "the page changed under the ref: snap again (the brain now retries by name)"),
    (r"nothing matches|nothing called|no field called|no field for", "the name was guessed: look first with snap -f <word> (the brain learns real names)"),
    (r"mouse or keyboard", "the user was busy; Lighting waits up to 10 s for a pause now"),
    (r"no window chosen", "name the app: lighting snap app:<name> (no windows call needed)"),
    (r"unknown command", "check the name in lighting help"),
    (r"did not answer|disconnected|not connected", "the browser was busy or restarted: lighting status"),
    (r"not there after|still there after|timed out|timeout after", "the page was slower than the wait: raise --timeout or wait for other text"),
    (r"matches several", "use a longer name or --first"),
    (r"needs --yes|irreversible", "a risky step: only after the user agreed, then --yes"),
    (r"no frame matching", "the frame was not loaded yet or its name was cut: Lighting waits 3 s and lists the frames now"),
    (r"runs as administrator", "an elevated window takes no clicks or keys from normal apps: lighting read / shot, or an admin terminal"),
    (r"needs |need a ", "the command was called without what it needs: see its line in lighting help"),
]
_b = {"data": None, "dirty": False, "saved": 0.0, "block": None}


def empty():
    return {"v": 1, "pages": {}, "fixes": {}, "stats": {"pruned": 0, "chars": 0, "regained": 0, "fixed": 0}}


def decay(age):
    return 0.5 ** (max(0.0, age) / (D.BRAIN_HALF_LIFE_D * 86400))


def norm(name):
    s = re.sub(r"\d+", "#", " ".join(str(name or "").lower().split()))
    return s.rstrip(".…").strip()[:60]


def page_key(url):
    try:
        u = urlsplit(url or "")
    except ValueError:
        return ""
    host = (u.hostname or "").lower()
    host = host[4:] if host.startswith("www.") else host
    if not host or u.scheme not in ("http", "https"):
        return ""
    segs = ["#" if re.fullmatch(r"\d+|[0-9a-f]{7,}|[\w-]{25,}", s, re.I) else s.lower()
            for s in [s for s in u.path.split("/") if s][:4]]
    return "/".join([host] + segs)


def site(ctx):
    if ctx.kind() == "app":
        try:
            from lighting import win
            return "app:" + win.exe_of(win.pid_of(ctx.target[1])).lower()
        except Exception:
            return ""
    return page_key((getattr(ctx, "where", None) or {}).get("url"))


def data():
    if _b["data"] is None:
        try:
            d = json.loads(D.BRAIN.read_text("utf-8"))
        except (OSError, ValueError):
            d = None
        if not isinstance(d, dict) or d.get("v") != 1:
            d = boot()
            _b["dirty"] = True
        _b["data"] = d
    return _b["data"]


def boot():
    d = empty()
    try:
        rows = (D.HOME / "episodes.jsonl").read_text("utf-8", errors="ignore").splitlines()
    except OSError:
        rows = []
    for row in rows:
        try:
            ep = json.loads(row)
        except ValueError:
            continue
        for s in ep.get("steps") or []:
            if s.get("cmd") not in LEARN_CMDS or not s.get("args"):
                continue
            w = s.get("where") or {}
            key = "app:" + (w.get("exe") or "").lower() if w.get("kind") == "app" else page_key(w.get("url"))
            for label in step_labels(s["cmd"], s["args"]):
                use(d, key, label, s.get("role"), s.get("t") or ep.get("t0") or time.time())
    return d


def step_labels(cmd, args):
    if cmd == "fill":
        return [a.split("=", 1)[0] for a in args if "=" in a and not ref_kind(a.split("=", 1)[0].strip())]
    a = args[0] if args else ""
    if not a or ref_kind(a) or a.startswith(("#", "url:", "css:", "@")) or a.lower() in ("focused", "screen"):
        return []
    return [a]


def use(d, key, label, role, t):
    n = norm(label)
    if not key or not n or key == "app:":
        return
    for k in dict.fromkeys([key, key.split("/")[0]]):
        p = d["pages"].setdefault(k, {"uses": 0, "t": t, "els": {}})
        p["uses"] += 1
        p["t"] = max(p["t"], t)
        e = p["els"].get(n)
        if e:
            e[0] = e[0] * decay(t - e[1]) + 1 if t >= e[1] else e[0] + decay(e[1] - t)
            e[1] = max(e[1], t)
            e[2] = role or e[2]
        else:
            p["els"][n] = [1.0, t, role, str(label)[:80]]


def ranked(key):
    d = data()
    p = d["pages"].get(key)
    if not p or p["uses"] < D.BRAIN_PAGE_USES:
        p = d["pages"].get(key.split("/")[0])
        if not p or p["uses"] < D.BRAIN_HOST_USES:
            return []
    now = time.time()
    return [k for k, _ in sorted(p["els"].items(), key=lambda kv: -kv[1][0] * decay(now - kv[1][1]))]


def can_prune(ctx):
    return bool(ctx.cfg.get("brain", True)) and not getattr(ctx, "no_prune", False)


def can_learn(ctx):
    return bool(ctx.cfg.get("brain", True)) and not getattr(ctx, "no_learn", False)


def prune(ctx, text, key):
    if not can_prune(ctx) or not key or not text:
        return text
    lines = text.split("\n")
    if len(lines) < 3 or any("since your last snap" in l or "unchanged since last snap" in l for l in lines[:3]):
        return text
    names = ranked(key)[: int(ctx.cfg.get("snap_top") or D.BRAIN_TOP)]
    if len(names) < D.BRAIN_MIN_HITS:
        return text
    exact, loose = set(names), [n for n in names if len(n) >= 4]
    keep, hidden, hits = [lines[0]], 0, 0
    for line in lines[1:]:
        m = ELEMENT.match(line)
        if not m:
            keep.append(line)
            continue
        n = norm(m.group(3))
        if m.group(2) in ALWAYS:
            keep.append(line)
        elif n and (n in exact or any(x in n for x in loose)):
            keep.append(line)
            hits += 1
        else:
            hidden += 1
    if hits < D.BRAIN_MIN_HITS or hidden < D.BRAIN_MIN_HIDDEN:
        return text
    keep.insert(1, "brain: %d controls you use here, %d others hidden (lighting snap --full shows all)" % (hits, hidden))
    out = "\n".join(keep)
    st = data()["stats"]
    st["pruned"] += 1
    st["chars"] += max(0, len(text) - len(out))
    trimmed(len(text) - len(out))
    _b["dirty"] = True
    return out


def scope_of(ctx):
    return (getattr(ctx, "where", None) or {}).get("tab") or ""


def remember_refs(ctx, out):
    if not isinstance(out, str) or '"' not in out:
        return
    lines = out.split("\n")
    m = HEADER.match(lines[0])
    scope = m.group(1) if m else scope_of(ctx)
    if not scope:
        return
    books = ctx.__dict__.setdefault("refnames", {})
    refs = None
    for line in lines:
        e = ELEMENT.match(line.lstrip("+ "))
        if not e or "." in e.group(1) or e.group(1)[0] != "e":
            continue
        if refs is None:
            refs = books.pop(scope, {})
            books[scope] = refs
        name = e.group(3)
        refs[e.group(1)] = (e.group(2), name, name.endswith(("...", "…")))
    while len(books) > 6:
        books.pop(next(iter(books)))
    if refs is not None and len(refs) > 3000:
        for k in list(refs)[:1000]:
            refs.pop(k)


def ref_name(ctx, ref):
    got = ctx.__dict__.get("refnames", {}).get(scope_of(ctx), {}).get(ref)
    if not got or got[2] or not got[1].strip():
        return None
    return got[0], got[1]


def fix_for(ctx, cmd, text):
    return data()["fixes"].get(site(ctx), {}).get(cmd + "|" + norm(text))


def alike(miss, label):
    a, b = norm(miss).strip("\"'"), norm(label)
    words = lambda s: {w for w in re.findall(r"\w+", s) if len(w) >= 3}
    return bool(words(a) & words(b)) or difflib.SequenceMatcher(None, a, b).ratio() >= D.BRAIN_FIX_ALIKE


def retry(ctx, name, pos, flags, err, route):
    if name not in TARGETED or not pos or not can_prune(ctx):
        return None
    text = err.text()
    kind = ref_kind(pos[0])
    if kind == "e" and "." not in pos[0] and GONE.search(text):
        got = ref_name(ctx, pos[0])
        if not got:
            return None
        role, label = got
        new_pos, new_flags = [label] + list(pos[1:]), dict(flags, role=role)
        try:
            out = route(ctx, name, new_pos, new_flags)
        except Fail:
            return None
        ctx.last_target = {"name": label, "role": role}
        count("regained")
        note(ctx, '%s was gone (the page changed); used the same %s "%s" instead' % (pos[0], role, label))
        return out, new_pos, new_flags
    if kind is None and MISS.search(text):
        fix = fix_for(ctx, name, pos[0])
        if not fix or fix[1] < D.BRAIN_FIX_SURE or not alike(pos[0], fix[0]):
            return None
        new_pos = [fix[0]] + list(pos[1:])
        try:
            out = route(ctx, name, new_pos, flags)
        except Fail:
            return None
        count("fixed")
        note(ctx, 'nothing called "%s" here; used "%s" (it worked here %d times after the same miss)' % (pos[0], fix[0], fix[1]))
        return out, new_pos, flags
    return None


def hint_for(ctx, name, pos, err):
    if name not in TARGETED or not pos or ref_kind(pos[0]) or not MISS.search(err.text()):
        return None
    fix = fix_for(ctx, name, pos[0])
    if not fix:
        return None
    return Fail('%s (worked here before after the same miss: "%s")' % (err, fix[0]), err.hint)


def count(key):
    st = data()["stats"]
    st[key] = st.get(key, 0) + 1
    _b["dirty"] = True


def blocked(key):
    now = time.time()
    if not _b["block"] or now - _b["block"][0] > 300:
        from lighting import browser
        _b["block"] = (now, browser.blocklist())
    host = key.split("/")[0]
    return any(host == b or host.endswith("." + b) for b in _b["block"][1])


def target_labels(ctx, name, pos, out):
    if name == "fill":
        return [(label, None) for label in step_labels(name, pos)]
    t = getattr(ctx, "last_target", None) or {}
    if t.get("name"):
        return [(t["name"], t.get("role"))]
    m = re.match(r"ok ((?:f\d+\.)?e\d+)\b", out or "")
    if m:
        got = ref_name(ctx, m.group(1))
        if got:
            return [(got[1], got[0])]
    return [(label, None) for label in step_labels(name, pos)]


def observe(ctx, name, pos, flags, out, ok, err=""):
    if ok and isinstance(out, str):
        remember_refs(ctx, out)
    if not can_learn(ctx) or name not in LEARN_CMDS or not pos:
        return
    key = site(ctx)
    if not key or blocked(key):
        return
    now = time.time()
    if not ok:
        if name in TARGETED and ref_kind(pos[0]) is None and MISS.search(err or ""):
            ctx.brain_miss = (key, name, pos[0], now)
        return
    d = data()
    labels = target_labels(ctx, name, pos, out)
    for label, role in labels:
        use(d, key, label, role, now)
    miss = getattr(ctx, "brain_miss", None)
    if miss and labels and miss[:2] == (key, name) and now - miss[3] < D.BRAIN_FIX_WINDOW_S and norm(labels[0][0]) != norm(miss[2]):
        fixes = d["fixes"].setdefault(key, {})
        k = name + "|" + norm(miss[2])
        old = fixes.get(k)
        fixes[k] = [labels[0][0], (old[1] + 1) if old and old[0] == labels[0][0] else 1, now]
    ctx.brain_miss = None
    _b["dirty"] = True
    if now - _b["saved"] > D.BRAIN_SAVE_S:
        save()


def save():
    d = _b["data"]
    if d is None or not _b["dirty"]:
        return
    pages, now = d["pages"], time.time()
    for k in sorted(pages, key=lambda k: pages[k]["t"])[: max(0, len(pages) - D.BRAIN_MAX_PAGES)]:
        del pages[k]
    for p in pages.values():
        if len(p["els"]) > D.BRAIN_MAX_ELS:
            p["els"] = dict(sorted(p["els"].items(), key=lambda kv: -kv[1][0] * decay(now - kv[1][1]))[: D.BRAIN_MAX_ELS])
    try:
        D.HOME.mkdir(parents=True, exist_ok=True)
        tmp = D.BRAIN.with_suffix(".tmp")
        tmp.write_text(json.dumps(d, ensure_ascii=False, separators=(",", ":")), "utf-8")
        tmp.replace(D.BRAIN)
        _b["dirty"], _b["saved"] = False, now
    except OSError:
        pass


def cmd_brain(ctx, pos, flags):
    d = data()
    sub = pos[0].lower() if pos else ""
    if sub == "reset":
        if not flags.get("yes"):
            raise Fail("this forgets everything Lighting learned about pages and apps",
                       "lighting brain reset --yes (only when the user asked for it)")
        _b["data"], _b["dirty"] = empty(), True
        save()
        return "brain reset: nothing learned any more"
    if sub == "forget":
        what = (pos[1] if len(pos) > 1 else "").lower().replace("https://", "").replace("http://", "").strip("/")
        if not what:
            raise Fail("forget needs a page, site or app", "lighting brain forget github.com | app:spotify.exe")
        gone = [k for k in d["pages"] if k == what or k.startswith(what + "/")]
        for k in gone:
            d["pages"].pop(k, None)
            d["fixes"].pop(k, None)
        _b["dirty"] = True
        save()
        return "forgot %d page%s" % (len(gone), "" if len(gone) == 1 else "s")
    pages, st = d["pages"], d["stats"]
    lines = ["brain: %d pages and apps, %d controls, %d fixes learned | %d snapshots trimmed (-%d tokens) | %d refs regained, %d fixes used" % (
        len(pages), sum(len(p["els"]) for p in pages.values()), sum(len(v) for v in d["fixes"].values()),
        st.get("pruned", 0), st.get("chars", 0) // 4, st.get("regained", 0), st.get("fixed", 0))]
    now = time.time()
    shown = [(k, p) for k, p in pages.items() if "/" in k or k.startswith("app:")]
    for k, p in sorted(shown, key=lambda kv: -kv[1]["uses"])[: int(flags.get("max") or 8)]:
        top = sorted(p["els"].values(), key=lambda v: -v[0] * decay(now - v[1]))[:5]
        lines.append("%s (%d uses): %s" % (k, p["uses"], ", ".join(v[3] for v in top)))
    for key, fx in list(d["fixes"].items())[:6]:
        for k, (to, n, _) in list(fx.items())[:2]:
            lines.append('fix on %s: %s "%s" -> "%s" (%dx)' % (key, k.split("|", 1)[0], k.split("|", 1)[1], to, n))
    lines.append("kept on this PC only (%s) | brain forget <site> | brain reset --yes | config brain off" % D.BRAIN.as_posix())
    return cap("\n".join(lines), "brain", lines=60)


def read_log(since=0.0):
    floor = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(since)) if since else ""
    rows = []
    for p in (D.LOG.with_suffix(".old.jsonl"), D.LOG):
        try:
            text = p.read_text("utf-8", errors="ignore")
        except OSError:
            continue
        for line in text.splitlines():
            if floor and line[7:26] < floor:
                continue
            try:
                r = json.loads(line)
                t = r["t"]
                r["ts"] = time.mktime((int(t[0:4]), int(t[5:7]), int(t[8:10]), int(t[11:13]), int(t[14:16]), int(t[17:19]), 0, 0, -1))
            except (KeyError, TypeError, ValueError):
                continue
            rows.append(r)
    return rows


def shape(err):
    s = re.sub(r'"[^"]*"', '"x"', err or "")
    s = re.sub(r"\b(?:f\d+\.)?[edwtfo]\d+\b", "ref", s)
    return re.sub(r"\d+", "n", s).strip()


def summary(days=7, now=None, rows=None):
    now = now or time.time()
    t0, t1 = now - days * 86400, now - 2 * days * 86400
    rows = read_log(t1) if rows is None else rows
    cur = [r for r in rows if r["ts"] >= t0 and not r.get("x")]
    prev = [r for r in rows if t1 <= r["ts"] < t0 and not r.get("x")]
    groups = {}
    for r in cur:
        if r.get("ok") is False:
            groups.setdefault(shape(r.get("e")), []).append(r)
    patterns = []
    for k, rs in sorted(groups.items(), key=lambda kv: -len(kv[1])):
        ts = sorted(r["ts"] for r in rs)
        burst = max((sum(1 for t in ts[i:] if t - ts[i] <= 600) for i in range(len(ts))), default=0)
        advice = next((a for p, a in ADVICE if re.search(p, k)), "look at lighting log for the details")
        patterns.append({"n": len(rs), "error": k, "cmds": sorted({r["cmd"] for r in rs}), "burst": burst, "advice": advice})
    fails = lambda rs: sum(1 for r in rs if r.get("ok") is False)
    return {"days": days, "runs": len(cur), "failed": fails(cur), "prev_runs": len(prev), "prev_failed": fails(prev),
            "patterns": patterns}


def pct(a, b):
    return 100.0 * a / b if b else 0.0


def cmd_improve(ctx, pos, flags):
    days = int(flags.get("days") or (pos[0] if pos and pos[0].isdigit() else 7))
    s = summary(days)
    if not s["runs"]:
        return "no commands in the last %d days" % days
    before = " (the %d days before: %.1f%%)" % (days, pct(s["prev_failed"], s["prev_runs"])) if s["prev_runs"] else ""
    lines = ["Lighting self-check, last %d days: %d commands, %.1f%% failed%s" % (days, s["runs"], pct(s["failed"], s["runs"]), before)]
    for i, p in enumerate(s["patterns"][:5], 1):
        lines.append("%d. %dx %s (%s) -> %s" % (i, p["n"], p["error"][:70], ", ".join(p["cmds"][:4]), p["advice"]))
        if p["burst"] >= 10:
            lines.append("   %d of them within 10 min: a loop kept going after the error" % p["burst"])
    d = data()
    st = d["stats"]
    lines.append("brain: %d pages known, %d fixes learned, %d snapshots trimmed (-%d tokens), %d refs regained, %d fixes used" % (
        sum(1 for k in d["pages"] if "/" in k or k.startswith("app:")), sum(len(v) for v in d["fixes"].values()),
        st.get("pruned", 0), st.get("chars", 0) // 4, st.get("regained", 0), st.get("fixed", 0)))
    return "\n".join(lines)


def weekly_tip(now=None):
    now = now or time.time()
    try:
        st = json.loads(D.IMPROVE.read_text("utf-8"))
    except (OSError, ValueError):
        st = {}
    if now - float(st.get("shown") or 0) < D.IMPROVE_EVERY_S:
        return ""
    st["shown"] = now
    try:
        D.IMPROVE.write_text(json.dumps(st), "utf-8")
    except OSError:
        pass
    s = summary(7, now)
    if s["failed"] < D.IMPROVE_MIN_FAILS or not s["patterns"]:
        return ""
    p = s["patterns"][0]
    return "\n".join(["Lighting self-check (weekly): %d commands in 7 days, %.0f%% failed." % (s["runs"], pct(s["failed"], s["runs"])),
                      "Most common: %dx %s -> %s" % (p["n"], p["error"][:60], p["advice"]),
                      "Details: lighting improve"])

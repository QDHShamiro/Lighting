import difflib
import json
import re
import threading
import time

from lighting import defaults as D
from lighting.common import Fail, cap, ref_kind

DIR = D.HOME / "routines"
EPISODES = D.HOME / "episodes.jsonl"
ACTIONS = {"open", "launch", "focus", "click", "type", "fill", "press", "select", "check", "hover", "scroll",
           "back", "forward", "reload", "wait", "expect", "dismiss", "dialog"}
ENTRY = {"open", "launch", "focus"}
REF_CMDS = {"click", "type", "select", "check", "hover"}
KEEP_FLAGS = {"new", "yes", "enter", "append", "submit", "first", "right", "double", "force", "until", "delta",
              "gone", "timeout", "game", "stay", "web", "max", "mouse", "text", "on"}
SHARED = {"click", "type", "press", "scroll", "hover"}
IDLE_S = 45
KEEP_EPISODES = 300
TRANSIENT = re.compile(r"nothing matches|nothing called|no field|is gone|no window matches|page changed|no tab yet|"
                       r"did not react|did not answer|not found|older snapshot|no match|nothing focused|"
                       r"still loading|navigated during", re.I)
TOKEN = re.compile(r"\w+|[^\w\s]", re.U)

_lock = threading.RLock()
_ep = {}


def _reset():
    _ep.clear()
    _ep.update({"steps": [], "cost": 0, "start": None, "t0": 0.0, "last": 0.0, "via": None, "lossy": False,
                "hinted": set(), "failed": None})


_reset()


def slug(text, n=3):
    words = [w for w in re.split(r"[^a-z0-9]+", (text or "").lower()) if w and not w.isdigit()]
    return "-".join(words[:n])


def enabled(ctx):
    return ctx.cfg.get("learn", True) and not getattr(ctx, "no_learn", False)


def site(where):
    if not where:
        return ""
    if where.get("kind") == "app":
        return (where.get("exe") or "").lower()
    m = re.match(r"^[a-z]+://([^/]+)", where.get("url") or "", re.I)
    return m.group(1).lower() if m else ""


def where_of(ctx):
    if ctx.kind() == "app":
        from lighting import win
        hwnd = ctx.target[1]
        if not win.alive(hwnd):
            return None
        return {"kind": "app", "exe": win.exe_of(win.pid_of(hwnd)), "title": win.text_of(hwnd)}
    w = getattr(ctx, "where", None)
    return dict(w) if w else None


def stable(ctx, name, pos, flags):
    args = list(pos)
    role = None
    if name in REF_CMDS and args and ref_kind(args[0]) in ("e", "d"):
        t = getattr(ctx, "last_target", None) or {}
        label = (t.get("name") or "").strip()
        if not label and t.get("hint"):
            label = "#" + t["hint"]
        if not label:
            return None
        args[0] = label[:80]
        role = t.get("role")
    if name == "type" and ctx.secret is not None:
        args = args[:1] + ["@secret"]
    kept = {k: v for k, v in flags.items() if k in KEEP_FLAGS}
    if name in SHARED and ctx.kind() in ("web", "app"):
        kept["on"] = ctx.kind()
    step = {"cmd": name, "args": args, "flags": kept}
    if role:
        step["role"] = role
    return step


def observe(ctx, name, pos, flags, out, ok):
    if not enabled(ctx):
        return
    now = time.time()
    with _lock:
        if _ep["steps"] and now - _ep["last"] > IDLE_S and not (_ep["via"] and _ep["failed"] is None):
            close(ctx)
        _ep["cost"] += len(out or "") // 4 + 15
        if name not in ACTIONS:
            return
        if not ok:
            if _ep["via"] and not _ep["failed"]:
                _ep["failed"] = len(_ep["steps"])
            return
        step = stable(ctx, name, pos, flags)
        if step is None:
            _ep["lossy"] = True
            return
        w = where_of(ctx)
        if (name in ENTRY and not (_ep["via"] and _ep["failed"] is None)
                and len([s for s in _ep["steps"] if s["cmd"] not in ENTRY]) >= 2 and site(w) != site(_ep["start"])):
            close(ctx)
        if not _ep["steps"]:
            _ep["start"] = w
            _ep["t0"] = now
        step["t"] = round(now, 2)
        step["where"] = w
        _ep["steps"].append(step)
        _ep["last"] = now
    hint(ctx)


def close(ctx):
    with _lock:
        ep = {"t0": _ep["t0"], "t1": _ep["last"], "start": _ep["start"], "steps": _ep["steps"], "cost": _ep["cost"],
              "via": _ep["via"], "failed": _ep["failed"], "lossy": _ep["lossy"], "params": _ep.get("params")}
        _reset()
    if len(ep["steps"]) < 2 or ep["lossy"]:
        return None
    past = episodes()
    persist(ep)
    try:
        return learn(ctx, ep, past)
    except Exception as e:
        ctx.d.events.append("routine learning failed: %s" % e)
        return None


def episodes(limit=150):
    try:
        rows = EPISODES.read_text("utf-8").splitlines()[-limit:]
    except OSError:
        return []
    out = []
    for r in rows:
        try:
            out.append(json.loads(r))
        except ValueError:
            pass
    return out


def persist(ep):
    D.HOME.mkdir(parents=True, exist_ok=True)
    with open(EPISODES, "a", encoding="utf-8") as f:
        f.write(json.dumps(ep, ensure_ascii=False) + "\n")
    try:
        rows = EPISODES.read_text("utf-8").splitlines()
        if len(rows) > KEEP_EPISODES + 100:
            EPISODES.write_text("\n".join(rows[-KEEP_EPISODES:]) + "\n", "utf-8")
    except OSError:
        pass


def text_of(step):
    s = "\x1f".join(step["args"])
    if step.get("flags"):
        s += "\x1f" + " ".join("--%s=%s" % kv for kv in sorted(step["flags"].items()))
    return s


def toks(s):
    return [m.group(0).lower() for m in TOKEN.finditer(s)]


def sim(a, b):
    if a["cmd"] != b["cmd"]:
        return 0.0
    ta, tb = toks(text_of(a)), toks(text_of(b))
    if not ta and not tb:
        return 1.0
    m = difflib.SequenceMatcher(None, ta, tb, autojunk=False)
    return 2.0 * sum(bl.size for bl in m.get_matching_blocks()) / (len(ta) + len(tb))


def local_align(A, B, simf):
    n, m = len(A), len(B)
    H = [[0.0] * (m + 1) for _ in range(n + 1)]
    P = [[0] * (m + 1) for _ in range(n + 1)]
    best, bi, bj = 0.0, 0, 0
    for i in range(1, n + 1):
        for j in range(1, m + 1):
            s = simf(A[i - 1], B[j - 1])
            diag = H[i - 1][j - 1] + (2.0 * s - 0.6 if s >= 0.5 else -1.0)
            up = H[i - 1][j] - 0.8
            left = H[i][j - 1] - 0.8
            h = max(0.0, diag, up, left)
            H[i][j] = h
            P[i][j] = 1 if h == diag and h > 0 else 2 if h == up and h > 0 else 3 if h == left and h > 0 else 0
            if h > best:
                best, bi, bj = h, i, j
    pairs = []
    i, j = bi, bj
    while i > 0 and j > 0 and H[i][j] > 0:
        p = P[i][j]
        if p == 1:
            pairs.append((i - 1, j - 1))
            i, j = i - 1, j - 1
        elif p == 2:
            i -= 1
        elif p == 3:
            pairs.append((None, j - 1))
            j -= 1
        else:
            break
    pairs.reverse()
    return best, pairs


def slot_pairs(a, b):
    sa, sb = "\x1f".join(a["args"]), "\x1f".join(b["args"])
    ta = [(m.group(0), m.start(), m.end()) for m in TOKEN.finditer(sa)]
    tb = [(m.group(0), m.start(), m.end()) for m in TOKEN.finditer(sb)]
    ops = difflib.SequenceMatcher(None, [t[0].lower() for t in ta], [t[0].lower() for t in tb], autojunk=False).get_opcodes()
    groups, cur = [], None
    for k, (op, i1, i2, j1, j2) in enumerate(ops):
        if op != "equal":
            if cur is None:
                cur = [i1, i2, j1, j2]
            else:
                cur[1], cur[3] = i2, j2
            continue
        glue = j2 - j1 <= 2 and all(not re.match(r"\w", t[0]) for t in tb[j1:j2])
        if cur is not None and glue and k + 1 < len(ops) and ops[k + 1][0] != "equal":
            cur[1], cur[3] = i2, j2
            continue
        if cur is not None:
            groups.append(cur)
            cur = None
    if cur is not None:
        groups.append(cur)
    out = []
    for i1, i2, j1, j2 in groups:
        if i2 <= i1 or j2 <= j1:
            continue
        va = sa[ta[i1][1]:ta[i2 - 1][2]]
        vb = sb[tb[j1][1]:tb[j2 - 1][2]]
        if "\x1f" in va or "\x1f" in vb or not re.search(r"\w", vb):
            continue
        prev = tb[j1 - 2][0] if j1 >= 2 and tb[j1 - 1][0] in (":", "=") and re.match(r"\w", tb[j1 - 2][0]) else ""
        out.append((va, vb, prev))
    return out


def substitute(step, mapping):
    if not mapping:
        return step
    args = []
    for a in step["args"]:
        for old, new in mapping.items():
            a = re.sub(r"(?<!\w)%s(?!\w)" % re.escape(old), new.replace("\\", "\\\\"), a)
        args.append(a)
    return dict(step, args=args)


def align(A, B):
    score, pairs = local_align(A, B, sim)
    if not pairs:
        return None
    mapping = {}
    for i, j in pairs:
        if i is None:
            continue
        s = sim(A[i], B[j])
        if 0.3 <= s < 1.0:
            for va, vb, _ in slot_pairs(A[i], B[j]):
                if len(va) <= 60 and len(vb) <= 60:
                    mapping[va] = vb
    if mapping:
        A2 = [substitute(s, mapping) for s in A]
        score2, pairs2 = local_align(A2, B, sim)
        if score2 >= score:
            score, pairs = score2, pairs2
    if score < 1.5:
        return None
    matched =[(i, j) for i, j in pairs if i is not None and sim(substitute(A[i], mapping), B[j]) >= 0.5]
    actions = [j for _, j in matched if B[j]["cmd"] not in ("wait", "scroll")]
    if len(actions) < 2:
        return None
    return {"score": score, "pairs": pairs, "mapping": mapping}


PARAM_BAD = {"https", "http", "www", "com", "de", "org", "net"}


def param_name(prev, step, value, used):
    base = ""
    if step["cmd"] == "fill":
        for a in step["args"]:
            label, _, v = a.partition("=")
            if value in v:
                base = slug(label, 2).replace("-", "_")
                break
    elif step["cmd"] == "type" and len(step["args"]) > 1 and value in " ".join(step["args"][1:]):
        base = slug(step["args"][0], 2).replace("-", "_")
    if not base and prev and prev.lower() not in PARAM_BAD and not prev.isdigit():
        base = prev.lower()
    if not base:
        base = {"click": "item", "open": "page", "launch": "app", "select": "option"}.get(step["cmd"], "text")
    base = re.sub(r"[^a-z0-9_]", "", base) or "text"
    if base[0].isdigit():
        base = "p" + base
    name, n = base, 2
    while name in used:
        name, n = "%s%d" % (base, n), n + 1
    return name


def templatize(steps, values):
    out = []
    for s in steps:
        args = []
        for a in s["args"]:
            for v, p in sorted(values.items(), key=lambda kv: -len(kv[0])):
                a = re.sub(r"(?<!\w)%s(?!\w)" % re.escape(v), "{%s}" % p, a)
            args.append(a)
        out.append(dict(s, args=args))
    return out


def fill(text, params):
    return re.sub(r"\{([a-z][a-z0-9_]*)\}", lambda m: params.get(m.group(1), m.group(0)), text)


def check_of(where, values):
    if not where:
        return None
    hay = (where.get("title") or "") + " " + (where.get("url") or "")
    has = ["{%s}" % p for v, p in values.items() if v.lower() in hay.lower()]
    if where.get("kind") == "app":
        return {"kind": "app", "exe": where.get("exe"), "has": has}
    url = re.sub(r"[?#].*$", "", where.get("url") or "")
    return {"kind": "web", "has": has, "url": None if has else url}


def entry_for(where):
    if not where:
        return None
    if where.get("kind") == "app":
        exe = re.sub(r"\.exe$", "", where.get("exe") or "", flags=re.I)
        if exe.lower() == "applicationframehost":
            return {"cmd": "launch", "args": [where.get("title") or ""], "flags": {}}
        return {"cmd": "focus", "args": ["app:" + exe], "flags": {}}
    if where.get("url"):
        return {"cmd": "open", "args": [where["url"]], "flags": {}}
    return None


def clean_step(s):
    out = {"cmd": s["cmd"], "args": list(s["args"]), "flags": dict(s.get("flags") or {})}
    for k in ("role", "optional"):
        if s.get(k):
            out[k] = s[k]
    return out


def build(A, ep, blk):
    B = ep["steps"]
    js = [j for _, j in blk["pairs"]]
    lo, hi = min(js), max(js)
    aligned = {j for i, j in blk["pairs"] if i is not None}
    steps = []
    for j in range(lo, hi + 1):
        s = clean_step(B[j])
        if j not in aligned:
            s["optional"] = True
        steps.append(s)
    if steps[0]["cmd"] not in ENTRY:
        e = entry_for(B[lo - 1]["where"] if lo > 0 else ep["start"])
        if e:
            steps.insert(0, e)
    values, used = {}, set()
    for i, j in blk["pairs"]:
        if i is None:
            continue
        for va, vb, prev in slot_pairs(A[i], B[j]):
            if vb in values or len(vb) > 60 or len(vb) < 2:
                continue
            p = param_name(prev, B[j], vb, used)
            used.add(p)
            values[vb] = p
    steps = templatize(steps, values)
    params = {p: v for v, p in values.items()}
    last_where = B[hi]["where"]
    return {"steps": steps, "params": params, "check": check_of(last_where, values)}


def auto_name(r):
    steps = r["steps"]
    first = steps[0]
    head = ""
    if first["cmd"] == "open":
        m = re.match(r"^(?:[a-z]+://)?(?:www\.)?([^/.]+)", first["args"][0], re.I)
        head = m.group(1) if m else ""
    elif first["args"]:
        head = first["args"][0].replace("app:", "").split(":")[0]
    tail = ""
    for s in reversed(steps):
        if s["cmd"] in ("click", "type", "fill", "select", "check") and s["args"]:
            tail = re.sub(r"\{[^}]*\}", " ", s["args"][0].split("=")[0])
            break
    name = slug(head + " " + tail, 4) or "routine"
    base, n = name, 2
    while (DIR / (name + ".json")).exists():
        name, n = "%s-%d" % (base, n), n + 1
    return name


def signature(steps):
    return json.dumps([(s["cmd"], s["args"]) for s in steps if not s.get("optional")], ensure_ascii=False)


def template_re(text):
    parts = re.split(r"(\{[a-z][a-z0-9_]*\})", text)
    rx = "".join("(?P<%s>.+?)" % p[1:-1] if re.fullmatch(r"\{[a-z][a-z0-9_]*\}", p) else re.escape(p) for p in parts)
    try:
        return re.compile("^" + rx + "$", re.S | re.I)
    except re.error:
        rx = "".join("(.+?)" if re.fullmatch(r"\{[a-z][a-z0-9_]*\}", p) else re.escape(p) for p in parts)
        return re.compile("^" + rx + "$", re.S | re.I)


def match_step(tpl, step):
    if tpl["cmd"] != step["cmd"] or len(tpl["args"]) != len(step["args"]):
        return None
    got = {}
    for ta, sa in zip(tpl["args"], step["args"]):
        m = template_re(ta).match(sa)
        if not m:
            return None
        got.update({k: v for k, v in m.groupdict().items() if v is not None})
    return got


def covers(r, steps):
    need = [s for s in r["steps"] if not s.get("optional")]
    k, got = 0, {}
    for s in steps:
        if k < len(need):
            m = match_step(need[k], s)
            if m is not None:
                got.update(m)
                k += 1
    return got if k == len(need) else None


def learn(ctx, ep, past):
    rs = load_all()
    if ep["via"]:
        return repair(ctx, ep, rs)
    for r in rs:
        if covers(r, ep["steps"]) is not None:
            r["stats"]["seen"] = r["stats"].get("seen", 0) + 1
            save(r)
            return None
    best, best_old = None, None
    for old in reversed(past):
        if old.get("via") or old.get("lossy"):
            continue
        blk = align(old["steps"], ep["steps"])
        if blk and (best is None or blk["score"] > best["score"]):
            best, best_old = blk, old
    if not best:
        return None
    body = build(best_old["steps"], ep, best)
    if any(signature(body["steps"]) == signature(r["steps"]) for r in rs):
        return None
    r = new_routine(body, "learned", (best_old.get("cost", 0) + ep["cost"]) // 2)
    r["name"] = auto_name(r)
    save(r)
    ctx.d.events.append("learned routine %s from 2 runs -> next time: %s" % (r["name"], usage(r)))
    return r


def repair(ctx, ep, rs):
    r = next((x for x in rs if x["name"] == ep["via"]), None)
    if not r or ep["failed"] is None:
        return None
    steps = [clean_step(s) for s in ep["steps"]]
    values = {v: p for p, v in (ep.get("params") or {}).items() if v}
    if not steps:
        return None
    r["steps"] = templatize(steps, values)
    r["check"] = check_of(ep["steps"][-1]["where"], values)
    r["version"] = r.get("version", 1) + 1
    r["stats"]["recent"] = []
    r["flaky"] = False
    save(r)
    ctx.d.events.append("repaired routine %s (v%d) from your fix" % (r["name"], r["version"]))
    return r


def new_routine(body, source, cost):
    return {"name": "", "version": 1, "source": source, "created": time.strftime("%Y-%m-%d %H:%M"),
            "params": body["params"], "steps": body["steps"], "check": body["check"], "cost": int(cost),
            "stats": {"runs": 0, "ok": 0, "fail": 0, "ms": 0, "saved": 0, "seen": 2 if source == "learned" else 1,
                      "recent": []}, "flaky": False}


def kv(p, v):
    v = str(v)
    return "%s=%s" % (p, v) if re.fullmatch(r"[\w.@:/+-]+", v) else "%s='%s'" % (p, v.replace("'", ""))


def usage(r, values=None):
    return "lighting run %s%s" % (r["name"], "".join(" " + kv(p, (values or {}).get(p, v)) for p, v in r["params"].items()))


def path(name):
    return DIR / (re.sub(r"[^a-z0-9_-]+", "-", name.lower()).strip("-") + ".json")


def save(r):
    DIR.mkdir(parents=True, exist_ok=True)
    path(r["name"]).write_text(json.dumps(r, indent=1, ensure_ascii=False), "utf-8")


def load_all():
    out = []
    if not DIR.exists():
        return out
    for p in sorted(DIR.glob("*.json")):
        try:
            out.append(json.loads(p.read_text("utf-8")))
        except (OSError, ValueError):
            pass
    return out


def load(name):
    p = path(name)
    if p.exists():
        return json.loads(p.read_text("utf-8"))
    rs = [r for r in load_all() if r["name"].startswith(name.lower())]
    if len(rs) == 1:
        return rs[0]
    if rs:
        raise Fail('"%s" matches %s' % (name, ", ".join(r["name"] for r in rs)), "use the full name")
    raise Fail('no routine "%s"' % name, "lighting routines")


def hint(ctx):
    with _lock:
        steps = list(_ep["steps"])
        hinted = _ep["hinted"]
        if _ep["via"]:
            return
    if not steps:
        return
    for r in load_all():
        if r.get("flaky") or r["name"] in hinted:
            continue
        need = r["steps"]
        for start in range(len(steps) - 1, -1, -1):
            got = match_step(need[0], steps[start])
            if got is None:
                continue
            k = 1
            ok = True
            for s in steps[start + 1:]:
                if k >= len(need):
                    ok = False
                    break
                m = match_step(need[k], s)
                if m is None:
                    if need[k].get("optional") and k + 1 < len(need) and match_step(need[k + 1], s) is not None:
                        k += 1
                        m = match_step(need[k], s)
                    if m is None:
                        ok = False
                        break
                got.update(m)
                k += 1
            if ok and k < len(need):
                hinted.add(r["name"])
                ctx.d.events.append("routine %s can finish this in one call: %s" % (r["name"], usage(r, got)))
                return
            break


def flag_argv(flags):
    out = []
    for k, v in flags.items():
        if v is True:
            out.append("--" + k)
        elif v not in (None, False, ""):
            out += ["--" + k, str(v)]
    return out


def step_line(s, params=None):
    args = [fill(a, params) if params else a for a in s["args"]]
    shown = " ".join('"%s"' % a if (" " in a or not a) else a for a in args)
    fl = " ".join(flag_argv({k: v for k, v in (s.get("flags") or {}).items() if k != "on"}))
    return (s["cmd"] + " " + shown + (" " + fl if fl else "")).strip() + (" (optional)" if s.get("optional") else "")


def verify(ctx, r, params):
    chk = r.get("check")
    w = where_of(ctx)
    if not chk or not w:
        return "", w
    hay = ((w.get("title") or "") + " " + (w.get("url") or "")).lower()
    if chk.get("has"):
        ok = all(fill(h, params).lower() in hay for h in chk["has"])
    elif chk.get("kind") == "app":
        ok = (w.get("exe") or "").lower() == (chk.get("exe") or "").lower()
    else:
        ok = (w.get("url") or "").startswith(chk.get("url") or "\x00")
    return ("verified" if ok else "not verified"), w


def place(w):
    if not w:
        return ""
    if w.get("kind") == "app":
        return "%s - %s" % ((w.get("title") or "")[:60], w.get("exe"))
    url = re.sub(r"^[a-z]+://(www\.)?", "", w.get("url") or "")
    return "%s - %s" % ((w.get("title") or "")[:60], url[:70])


def record_result(r, ok, ms, out_tokens, err=None):
    st = r["stats"]
    st["runs"] += 1
    st["ok" if ok else "fail"] += 1
    st["ms"] = int(ms) if not st.get("ms") else int(st["ms"] * 0.7 + ms * 0.3)
    if ok:
        st["saved"] += max(0, r.get("cost", 0) - out_tokens)
    st["recent"] = (st.get("recent", []) + [1 if ok else 0])[-10:]
    st["last"] = time.strftime("%Y-%m-%d %H:%M")
    st["last_error"] = err
    last5 = st["recent"][-5:]
    r["flaky"] = len(last5) >= 3 and last5.count(0) >= 3
    save(r)


def cmd_run(ctx, pos, flags):
    from lighting.commands import run_one
    if not pos:
        raise Fail("run needs a routine name", "lighting routines")
    r = load(pos[0])
    params = dict(r["params"]) if flags.get("defaults") else {}
    for p in pos[1:]:
        k, sep, v = p.partition("=")
        if not sep:
            raise Fail("parameters look like name=value, got %s" % p)
        params[k.strip()] = v
    missing = [p for p in r["params"] if p not in params]
    if missing:
        raise Fail("%s needs %s" % (r["name"], ", ".join(missing)), usage(r))
    if any("@secret" in a for s in r["steps"] for a in s["args"]) and ctx.secret is None:
        raise Fail("%s types a secret" % r["name"], "lighting run %s ... --env VAR" % r["name"])
    with _lock:
        if _ep["steps"]:
            close(ctx)
        _ep["via"] = r["name"]
        _ep["params"] = params
    t0 = time.time()
    n = len(r["steps"])
    skipped = 0
    for i, s in enumerate(r["steps"], 1):
        if ctx.d.abort.is_set():
            raise Fail("%s stopped by hotkey at step %d/%d" % (r["name"], i, n))
        if s.get("flags", {}).get("yes") and not flags.get("yes"):
            record_result(r, False, (time.time() - t0) * 1000, 0, "needs --yes")
            raise Fail("%s step %d/%d (%s) needs a confirmation" % (r["name"], i, n, step_line(s, params)),
                       "rerun with --yes if the user wants this")
        argv = [s["cmd"]] + [fill(a, params) for a in s["args"]] + flag_argv(s.get("flags") or {})
        limit = float((s.get("flags") or {}).get("timeout") or 0) / 1000 or 6.0
        deadline = time.time() + min(max(limit, 4.0), 30.0)
        while True:
            try:
                run_one(ctx, argv)
                break
            except Fail as e:
                msg = str(e)
                if "matches several" in msg and "--first" not in argv:
                    argv.append("--first")
                    continue
                if s.get("optional"):
                    skipped += 1
                    break
                if TRANSIENT.search(msg) and time.time() < deadline:
                    time.sleep(0.3)
                    continue
                ms = (time.time() - t0) * 1000
                msg = re.sub(r" -> try: .*$", "", msg)
                record_result(r, False, ms, 0, "step %d: %s" % (i, msg[:120]))
                raise Fail("%s stopped at step %d/%d (%s): %s" % (r["name"], i, n, step_line(s, params), msg),
                           "the steps before are done; finish by hand (lighting snap), Lighting learns the fix")
    ms = (time.time() - t0) * 1000
    status, w = verify(ctx, r, params)
    with _lock:
        close(ctx)
    if status == "not verified":
        record_result(r, False, ms, 0, "result not verified")
        raise Fail("%s ran all %d steps, but the result does not look right: %s" % (r["name"], n - skipped, place(w)),
                   "check with lighting snap; wrong parameter or the site changed")
    line = "ok %s: %d steps in %.1f s%s%s" % (r["name"], n - skipped, ms / 1000, ", " + status if status else "",
                                              " -> " + place(w) if w else "")
    if w and not (r.get("check") or {}).get("has"):
        better = check_of(w, {v: p for p, v in params.items() if v and len(v) >= 2})
        if better and better.get("has"):
            r["check"] = better
    record_result(r, True, ms, len(line) // 4 + 15)
    return line


def cmd_routines(ctx, pos, flags):
    rs = load_all()
    if flags.get("f"):
        from lighting.common import terms, tiers
        ws = terms(flags["f"])
        rs = [r for r in rs if any(tiers(ws, r["name"], " ".join(" ".join(s["args"]) for s in r["steps"])))]
    if not rs:
        return "no routines yet (Lighting learns one when a task repeats; lighting routine save <name> saves the last task)"
    lines = []
    for r in sorted(rs, key=lambda x: -x["stats"].get("runs", 0)):
        st = r["stats"]
        rate = "%d/%d ok" % (st["ok"], st["runs"]) if st["runs"] else "new"
        extra = ", %.1f s" % (st["ms"] / 1000) if st.get("ms") else ""
        saved = ", saved ~%s tokens" % short_num(st["saved"]) if st.get("saved") else ""
        flaky = " FLAKY" if r.get("flaky") else ""
        args = "".join(" %s=" % p for p in r["params"])
        steps = " > ".join(step_line(s) for s in r["steps"] if not s.get("optional"))
        lines.append("%s%s | %s%s%s%s | %s" % (r["name"], args, rate, extra, saved, flaky, steps[:110]))
    return cap("\n".join(lines), "routines", lines=60)


def short_num(n):
    return "%.1fk" % (n / 1000.0) if n >= 1000 else str(n)


def cmd_routine(ctx, pos, flags):
    sub = pos[0].lower() if pos else "list"
    rest = pos[1:]
    if sub in ("list", "ls"):
        return cmd_routines(ctx, rest, flags)
    if sub == "show":
        r = load(need(rest, "show"))
        st = r["stats"]
        out = ["%s v%d (%s, %s)%s" % (r["name"], r.get("version", 1), r["source"], r["created"], " FLAKY" if r.get("flaky") else "")]
        if r["params"]:
            out.append("params: " + ", ".join('%s (last "%s")' % kv for kv in r["params"].items()))
        out += ["%d %s" % (i, step_line(s)) for i, s in enumerate(r["steps"], 1)]
        out.append("runs %d, ok %d, fail %d, ~%d ms, saved ~%d tokens, manual cost ~%d tokens%s" % (
            st["runs"], st["ok"], st["fail"], st.get("ms", 0), st.get("saved", 0), r.get("cost", 0),
            ", last error: " + st["last_error"] if st.get("last_error") else ""))
        return "\n".join(out)
    if sub in ("rm", "delete"):
        r = load(need(rest, "rm"))
        path(r["name"]).unlink()
        return "removed routine %s" % r["name"]
    if sub == "rename":
        if len(rest) < 2:
            raise Fail("rename needs old and new name", "lighting routine rename old new")
        r = load(rest[0])
        new = slug(rest[1], 6) or rest[1]
        if path(new).exists():
            raise Fail("a routine %s exists already" % new)
        path(r["name"]).unlink()
        r["name"] = new
        save(r)
        return "renamed to %s" % new
    if sub == "save":
        return save_current(ctx, need(rest, "save"), rest[1:], flags)
    if sub == "forget":
        try:
            EPISODES.unlink()
        except OSError:
            pass
        with _lock:
            _reset()
        return "task history cleared (routines stay)"
    if sub == "learn":
        r = close(ctx)
        return "learned %s" % r["name"] if r else "nothing new to learn"
    raise Fail("unknown routine command %s" % sub, "lighting routine list|show|save|rm|rename|forget")


def need(rest, what):
    if not rest:
        raise Fail("routine %s needs a name" % what)
    return rest[0]


def from_steps(name, raw, kv, cost, source, start=None):
    steps = [clean_step(s) for s in raw]
    if steps and steps[0]["cmd"] not in ENTRY:
        e = entry_for(start)
        if e:
            steps.insert(0, e)
    values = {}
    for p in kv:
        k, sep, v = p.partition("=")
        if sep and v:
            values[v] = re.sub(r"[^a-z0-9_]", "", k.lower()) or "text"
    r = new_routine({"steps": templatize(steps, values), "params": {p: v for v, p in values.items()},
                     "check": check_of(raw[-1].get("where") if raw else None, values)}, source, cost)
    r["name"] = slug(name, 6) or name
    save(r)
    return r


def save_current(ctx, name, kv, flags):
    with _lock:
        steps = list(_ep["steps"])
        cost = _ep["cost"]
        start = _ep["start"]
    if not steps:
        past = [e for e in episodes(5) if not e.get("via")]
        if not past:
            raise Fail("no finished task to save", "do the task with lighting first")
        steps, cost, start = past[-1]["steps"], past[-1]["cost"], past[-1]["start"]
    if flags.get("last"):
        steps = steps[-int(flags["last"]):]
    r = from_steps(name, steps, kv, cost, "saved", start)
    with _lock:
        _reset()
    return "saved routine %s (%d steps) -> %s" % (r["name"], len(r["steps"]), usage(r))

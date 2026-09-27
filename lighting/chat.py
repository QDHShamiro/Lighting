import hashlib
import json
import re
import time

from lighting import defaults as D
from lighting import win
from lighting.common import Fail, Pending, parse_ms, ref_kind

LIST, ITEM, BUTTON, TEXT, LINK, IMAGE, EDIT, DOC = 50008, 50007, 50000, 50020, 50005, 50006, 50004, 50030
OWN_ACTIONS = {"bearbeiten", "edit", "modifier", "editar", "modifica", "bewerken", "edytuj"}
LIST_NAME = re.compile(r"^(nachrichten|messages|chat|unterhaltung|conversation|verlauf|messaggi|mensajes)", re.I)
FIELD_NAME = re.compile(r"(nachricht|message|schreib|write|antwort|reply|mensaje|messaggio)", re.I)
TIME_RE = re.compile(r"\b(\d{1,2}:\d{2})(?:\s*[ap]\.?m\.?)?\b", re.I)
DAY_RE = re.compile(r"\d{4}|heute|gestern|today|yesterday", re.I)
LINK_RE = re.compile(r"https?://|www\.|discord\.gg/|\b[\w-]+\.(com|de|net|org|gg|io|me|ly|to)/", re.I)
MEMO = D.HOME / "chat.json"
_chats = {}
_conds = {}


def api():
    from lighting import uia
    return uia.api()


def cond(ctype):
    if ctype not in _conds:
        u, U = api()
        _conds[ctype] = u.CreateTrueCondition() if ctype is None else u.CreatePropertyCondition(U.UIA_ControlTypePropertyId, ctype)
    return _conds[ctype]


def props(el):
    from lighting import uia
    U = api()[1]
    return (uia.cached(el, U.UIA_ControlTypePropertyId), " ".join((uia.cached(el, U.UIA_NamePropertyId) or "").split()),
            uia.cached(el, U.UIA_AutomationIdPropertyId) or "")


def find_all(el, scope, ctype):
    from lighting import uia
    arr = el.FindAllBuildCache(scope, cond(ctype), uia._state["cache"])
    return [arr.GetElement(i) for i in range(arr.Length)]


def area(el):
    from lighting import uia
    l, t, r, b = uia.rect_of(el)
    return max(0, r - l) * max(0, b - t)


def memo():
    try:
        return json.loads(MEMO.read_text("utf-8"))
    except (OSError, ValueError):
        return {}


def remember_me(exe, name):
    data = memo()
    if data.get(exe, {}).get("me") == name:
        return
    data.setdefault(exe, {})["me"] = name
    D.HOME.mkdir(parents=True, exist_ok=True)
    MEMO.write_text(json.dumps(data, indent=1, ensure_ascii=False), "utf-8")


def chat(hwnd):
    return _chats.setdefault(hwnd, {"sent": [], "unanswered": 0, "sent_t": 0.0, "base": None, "parsed": {}})


def resolve(ctx, spec):
    from lighting import desktop
    if not spec:
        return desktop.target(ctx)
    if ref_kind(spec) == "w" or spec.lower().startswith("app:"):
        hwnd = desktop.find_window(ctx, spec)
    else:
        hits = [w for w in win.windows() if spec.lower() in w["title"].lower()]
        hits.sort(key=lambda w: ((w["exe"] or "").lower() in desktop.BROWSER_EXES, not w["fg"]))
        if not hits:
            raise Fail('no window matches "%s"' % spec, "lighting windows -f chat")
        hwnd = hits[0]["hwnd"]
    desktop.state(ctx).hwnd = hwnd
    ctx.target = ("app", hwnd)
    return hwnd


def message_list(hwnd):
    root = api()[0].ElementFromHandle(hwnd)
    lists = find_all(root, 4, LIST)
    if not lists:
        raise Fail("no message list in %s" % win.text_of(hwnd)[:50],
                   "lighting focus w<N> (a minimized window may hide it), or lighting snap w<N>")
    return max(lists, key=lambda el: area(el) * (10 if LIST_NAME.match(props(el)[1]) else 1))


def items(hwnd):
    out = []
    for el in find_all(message_list(hwnd), 2, ITEM):
        _, name, aid = props(el)
        m = re.search(r"(\d{6,})$", aid)
        key = aid or "h" + hashlib.md5(name.encode("utf-8", "replace")).hexdigest()[:12]
        out.append({"key": key, "num": int(m.group(1)) if m else None, "el": el, "name": name})
    return out


def baseline(hwnd, rows=None):
    rows = items(hwnd) if rows is None else rows
    nums = [r["num"] for r in rows if r["num"] is not None]
    return {"keys": {r["key"] for r in rows}, "top": max(nums) if nums else None,
            "last": rows[-1]["key"] if rows else None, "t": time.time()}


def fresh_rows(base, rows):
    if base["top"] is not None and all(r["num"] is not None for r in rows):
        return [r for r in rows if r["num"] > base["top"]]
    keys = [r["key"] for r in rows]
    start = keys.index(base["last"]) + 1 if base["last"] in keys else 0
    return [r for r in rows[start:] if r["key"] not in base["keys"]]


def is_stamp(text):
    return len(text) < 60 and bool(TIME_RE.search(text)) and (bool(DAY_RE.search(text)) or TIME_RE.fullmatch(text.strip()) is not None)


def parse(row):
    desc = find_all(row["el"], 4, None)
    sender, when, parts, own, stamp, toolbar = None, None, [], False, False, False
    for d in desc:
        ct, name, aid = props(d)
        if aid.startswith("message-timestamp"):
            stamp = True
            continue
        if ct == BUTTON:
            if name.lower() in OWN_ACTIONS:
                own = True
            if stamp:
                toolbar = True
            elif sender is None and name:
                sender = name
            continue
        if not name or toolbar:
            continue
        if not stamp or is_stamp(name):
            hit = TIME_RE.search(name)
            if hit:
                when = when or hit.group(1)
            continue
        if ct in (TEXT, LINK) and (not parts or parts[-1] != name):
            parts.append(name)
        elif ct == IMAGE:
            parts.append("[image %s]" % name)
    if not stamp:
        return {"sender": None, "when": None, "text": row["name"], "own": own}
    return {"sender": sender, "when": when, "text": " ".join(parts), "own": own}


def parsed(st, row):
    got = st["parsed"].get(row["key"])
    if got is None:
        got = parse(row)
        st["parsed"][row["key"]] = got
        if len(st["parsed"]) > 400:
            st["parsed"].clear()
    return got


def short_sender(name):
    return re.sub(r"\s*\([^)]*\)\s*$", "", name or "").strip() or "?"


def norm(text):
    return re.sub(r"\s+", " ", text or "").strip().lower()


def said_by_me(st, text):
    t = norm(text)
    return any(t == s or (len(s) >= 3 and t.endswith(" " + s)) for s in st["sent"])


def news(ctx, hwnd, base, who):
    st = chat(hwnd)
    rows = items(hwnd)
    new = fresh_rows(base, rows)
    if not new:
        return [], rows
    first = rows.index(new[0])
    sender = None
    for row in reversed(rows[max(0, first - 10):first]):
        sender = parsed(st, row)["sender"]
        if sender:
            break
    me = memo().get(win.exe_of(win.pid_of(hwnd)).lower(), {}).get("me")
    out = []
    for row in new:
        m = dict(parsed(st, row))
        sender = m["sender"] = m["sender"] or sender
        mine = m["own"] or (me and short_sender(sender).lower() == me.lower()) or said_by_me(st, m["text"])
        if mine or (who and who not in (sender or "").lower()):
            continue
        m["key"], m["num"] = row["key"], row["num"]
        out.append(m)
    return out, rows


def label_id(row):
    return str(row["num"]) if row.get("num") is not None else row["key"]


def show(ctx, hwnd, msgs):
    from lighting import desktop
    head = "new [%s] %s #%s" % (desktop.wref(ctx, hwnd), win.text_of(hwnd)[:50], label_id(msgs[-1]))
    lines = []
    for m in msgs[-D.CHAT_LINES:]:
        text = m["text"] if len(m["text"]) <= D.CHAT_TEXT else m["text"][:D.CHAT_TEXT - 3] + "..."
        who = "%s%s: " % (short_sender(m["sender"]), " " + m["when"] if m["when"] else "") if m["sender"] else ""
        lines.append(who + text)
    more = len(msgs) - D.CHAT_LINES
    if more > 0:
        lines.insert(0, "... +%d older (lighting inbox %s --since %s)" % (more, desktop.wref(ctx, hwnd), label_id(msgs[0])))
    return head + "\n" + "\n".join(lines)


def human(seconds):
    return "%dm" % (seconds // 60) if seconds >= 120 and seconds % 60 == 0 else "%d s" % seconds


def keep(ctx, hwnd):
    from lighting import desktop
    desktop.state(ctx).launched.pop(hwnd, None)


def watch(ctx, hwnd, base, who, total, prefix=""):
    st = chat(hwnd)

    def check(last):
        if not win.alive(hwnd):
            raise Fail("the chat window was closed", "lighting windows -f chat")
        msgs, rows = news(ctx, hwnd, base, who)
        if msgs:
            st["base"] = baseline(hwnd, rows)
            st["unanswered"] = 0
            return prefix + show(ctx, hwnd, msgs)
        if last:
            tail = label_id(rows[-1]) if rows else "-"
            return prefix + "nothing new in %s (last #%s)" % (human(total), tail)
        return None

    return check


def cmd_inbox(ctx, pos, flags):
    hwnd = resolve(ctx, pos[0] if pos else None)
    keep(ctx, hwnd)
    st = chat(hwnd)
    total = parse_ms(flags.get("timeout"), D.LONG_WAITS["inbox"]) / 1000.0
    since = flags.get("since")
    if since:
        digits = re.sub(r"\D", "", str(since))
        base = {"keys": set(), "top": int(digits) if digits else None, "last": str(since), "t": time.time()}
    elif st["base"] and time.time() - st["base"]["t"] < 900:
        base = st["base"]
    else:
        base = baseline(hwnd)
    check = watch(ctx, hwnd, base, (flags.get("from") or "").lower(), total)
    res = check(total <= 0)
    return res if res is not None else Pending(check, total, D.CHAT_POLL_S)


def composer(hwnd):
    root = api()[0].ElementFromHandle(hwnd)
    wl, wt, wr, wb = win.rect(hwnd)
    best = None
    for el in find_all(root, 4, EDIT) + find_all(root, 4, DOC):
        from lighting import uia
        ct, name, aid = props(el)
        l, t, r, b = uia.rect_of(el)
        if aid == "RootWebArea" or (r - l) * (b - t) <= 0 or (ct == DOC and (b - t) > (wb - wt) * 0.5):
            continue
        if re.search(r"such|search|filter|find", name, re.I):
            continue
        score = (2 if FIELD_NAME.search(name) else 0, b)
        if best is None or score > best[0]:
            best = (score, el, name)
    if best is None:
        raise Fail("no message field in %s" % win.text_of(hwnd)[:50], "lighting snap w<N> -f nachricht, then type d<N> text --enter")
    return best[1], best[2] or "message field"


def confirm(ctx, hwnd, base, text):
    st = chat(hwnd)
    mine = {"sent": [norm(text)]}
    deadline = time.time() + 3.0
    while time.time() < deadline:
        rows = items(hwnd)
        for row in reversed(fresh_rows(base, rows)):
            m = parsed(st, row)
            if said_by_me(mine, m["text"]):
                sender = m["sender"]
                if not sender:
                    for prev in reversed(rows[:rows.index(row)][-10:]):
                        sender = parsed(st, prev)["sender"]
                        if sender:
                            break
                if sender and m["own"]:
                    remember_me(win.exe_of(win.pid_of(hwnd)).lower(), short_sender(sender))
                return label_id(row)
        time.sleep(0.25)
    return None


def cmd_reply(ctx, pos, flags):
    from lighting import desktop
    if len(pos) < 2:
        raise Fail("reply needs the chat and the text", 'lighting reply "@Name" "text" [--wait]')
    hwnd = resolve(ctx, pos[0])
    text = ctx.secret if ctx.secret is not None else " ".join(pos[1:])
    if not text.strip():
        raise Fail("nothing to send")
    st = chat(hwnd)
    if LINK_RE.search(text) and not flags.get("yes"):
        raise Fail("the message contains a link", "send it only if the user asked for it: lighting reply ... --yes")
    if st["unanswered"] >= D.CHAT_UNANSWERED and not flags.get("yes"):
        raise Fail("%d messages in a row without an answer" % st["unanswered"],
                   "wait for an answer (lighting inbox %s) or ask the user; --yes sends anyway" % desktop.wref(ctx, hwnd))
    gap = D.CHAT_GAP_S - (time.time() - st["sent_t"])
    if gap > 0:
        time.sleep(gap)
    keep(ctx, hwnd)
    base = baseline(hwnd)
    el, label = composer(hwnd)
    desktop.put_text(ctx, hwnd, el, label, text, {"enter": True, "stay": flags.get("stay")})
    st["sent_t"] = time.time()
    st["unanswered"] += 1
    st["sent"] = (st["sent"] + [norm(text)])[-20:]
    st["base"] = base
    mid = confirm(ctx, hwnd, base, text)
    head = "sent to [%s] %s%s" % (desktop.wref(ctx, hwnd), win.text_of(hwnd)[:50], " #" + mid if mid else " (not visible yet)")
    if not flags.get("wait"):
        return head
    total = parse_ms(flags.get("timeout"), D.LONG_WAITS["reply"]) / 1000.0
    check = watch(ctx, hwnd, base, (flags.get("from") or "").lower(), total, head + "\n")
    res = check(False)
    return res if res is not None else Pending(check, total, D.CHAT_POLL_S)

import os
import re
import time

from lighting import defaults as D
from lighting import pointer
from lighting import win
from lighting.common import Fail, best_only, cap, ref_kind, terms, tiers

BROWSER_EXES = {"brave.exe", "chrome.exe", "msedge.exe", "opera.exe", "vivaldi.exe"}
CHROMIUM_CLASS = "Chrome_WidgetWin_1"
TERMINALS = {"CASCADIA_HOSTING_WINDOW_CLASS", "ConsoleWindowClass", "mintty", "VirtualConsoleClass", "PuTTY"}


class AppState:
    def __init__(self):
        self.windows = {}
        self.refs = {}
        self.ids = {}
        self.seq = 0
        self.ocr = {}
        self.hwnd = None
        self.snap_t = 0.0
        self.launched = {}
        self.front0 = None
        self.slow = set()
        self.last_snap = {}
        self.full = None


def state(ctx):
    if not hasattr(ctx, "app"):
        ctx.app = AppState()
    return ctx.app


def list_windows(ctx):
    st = state(ctx)
    wins = win.windows()
    known = {v: k for k, v in st.windows.items()}
    lines = []
    for w in wins:
        ref = known.get(w["hwnd"])
        if not ref:
            ref = "w%d" % (len(st.windows) + 1)
            while ref in st.windows:
                ref = "w%d" % (int(ref[1:]) + 1)
            st.windows[ref] = w["hwnd"]
        flag = " *" if w["fg"] else ""
        flag += " (min)" if w["min"] else ""
        title = w["title"] if len(w["title"]) <= 60 else w["title"][:57] + "..."
        lines.append((int(ref[1:]), "%s %s - %s%s" % (ref, title, w["exe"] or w["cls"], flag)))
    lines.sort()
    return [l for _, l in lines]


def find_window(ctx, spec):
    st = state(ctx)
    if ref_kind(spec) == "w":
        hwnd = st.windows.get(spec)
        if not hwnd:
            list_windows(ctx)
            hwnd = st.windows.get(spec)
        if not hwnd or not win.alive(hwnd):
            raise Fail("window %s is gone" % spec, "lighting windows")
        return hwnd
    text = spec[4:] if spec.lower().startswith("app:") else spec
    tl = text.lower()
    best = None
    for w in win.windows():
        title, exe = w["title"].lower(), (w["exe"] or "").lower()
        score = 3 if title == tl else 2 if exe.replace(".exe", "") == tl else 1 if tl in title or tl in exe else 0
        if score and (not best or score > best[0]):
            best = (score, w["hwnd"])
    if not best:
        raise Fail('no window matches "%s"' % text, "lighting windows")
    list_windows(ctx)
    return best[1]


def target(ctx, pos=None):
    st = state(ctx)
    if pos and (ref_kind(pos[0]) == "w" or pos[0].lower().startswith("app:")):
        hwnd = find_window(ctx, pos[0])
    elif st.hwnd and win.alive(st.hwnd):
        hwnd = st.hwnd
    else:
        raise Fail("no window chosen", "lighting windows, then lighting snap w<N>")
    st.hwnd = hwnd
    ctx.target = ("app", hwnd)
    return hwnd


def wref(ctx, hwnd):
    st = state(ctx)
    for k, v in st.windows.items():
        if v == hwnd:
            return k
    list_windows(ctx)
    for k, v in st.windows.items():
        if v == hwnd:
            return k
    return "w?"


def header(ctx, hwnd, extra=""):
    pid = win.pid_of(hwnd)
    title = win.text_of(hwnd)
    title = title if len(title) <= 70 else title[:67] + "..."
    return "[%s] %s - %s%s" % (wref(ctx, hwnd), title, win.exe_of(pid), extra)


def is_browser(hwnd):
    return win.class_of(hwnd) == CHROMIUM_CLASS and win.exe_of(win.pid_of(hwnd)).lower() in BROWSER_EXES


BIDI = re.compile("[‎‏‪-‮⁦-⁩]")


def inside(a, b):
    return a[1] >= b[1] - 2 and a[3] <= b[3] + 2 and a[0] >= b[0] - 2


def snapshot(ctx, hwnd, flags):
    from lighting import uia
    st = state(ctx)
    if win.elevated(win.pid_of(hwnd)):
        raise Fail("this window runs as administrator; Windows blocks control from normal apps",
                   "start the terminal as administrator, or use lighting read / shot")
    skip = None
    if is_browser(hwnd) and not flags.get("web"):
        root = uia.api()[0].ElementFromHandle(hwnd)
        skip = uia.document_rect(root)
    root, items = uia.collect(hwnd, with_text=bool(flags.get("text")), skip_rect=skip)
    if len(items) < 3 and win.class_of(hwnd) == CHROMIUM_CLASS:
        time.sleep(0.4)
        root, items = uia.collect(hwnd, with_text=bool(flags.get("text")), skip_rect=skip)
    from lighting import keys
    U = uia.api()[1]
    filt = terms(flags.get("f"))
    lines, refs, row, ft, pairs = [], {}, None, {}, []
    for el in items:
        role, name, extra = uia.describe(el)
        name, extra = BIDI.sub("", name), BIDI.sub("", extra)
        ak = uia.cached(el, U.UIA_AcceleratorKeyPropertyId) or uia.cached(el, U.UIA_AccessKeyPropertyId)
        hinted = keys.from_name(name)
        if hinted:
            pairs.append(hinted)
            name = hinted[1]
        elif ak and name:
            pairs.append((ak, name))
        if role == "text" and (not name or len(name) > 120):
            continue
        if not name and role in ("listitem", "dataitem", "treeitem", "tab", "button", "link", "menuitem") and not extra:
            continue
        rid = uia.runtime_id(el)
        key = rid or (role, name, uia.rect_of(el))
        ref = st.ids.get(key)
        if not ref:
            st.seq += 1
            ref = "d%d" % st.seq
            st.ids[key] = ref
        refs[ref] = el
        if row and role in ("edit", "text") and inside(uia.rect_of(el), row[1]):
            m = re.search(r'="([^"]*)"', extra)
            val = m.group(1) if m else (name if role == "text" else "")
            if val and val != row[2] and len(lines[row[0]]) < 160:
                lines[row[0]] += " | " + val
            continue
        row = None
        t = tiers(filt, name, role, extra) if filt else None
        if t is not None and not any(t):
            continue
        ft[len(lines)] = t
        if role == "text":
            lines.append("  %s" % name)
            continue
        label = name if len(name) <= 60 else name[:57] + "..."
        lines.append("%s %s%s%s" % (ref, role, ' "%s"' % label.replace('"', "'") if label else "", extra))
        if role in ("listitem", "dataitem"):
            row = (len(lines) - 1, uia.rect_of(el), name)
    st.refs = refs
    st.snap_t = time.time()
    st.snap_hwnd = hwnd
    if pairs:
        keys.harvest(win.exe_of(win.pid_of(hwnd)).lower(), pairs)
    folded = 0
    if filt:
        lines = best_only([(ft.get(i), l) for i, l in enumerate(lines)])
    else:
        if not flags.get("text"):
            st.full = (hwnd, time.time(), list(lines))
        if not flags.get("all"):
            lines, folded = fold(lines)
    head = header(ctx, hwnd, " (%d controls%s)" % (len(refs), ", browser page hidden: use lighting snap for the page" if skip else ""))
    if not lines:
        lines.append("(no controls found: try lighting read %s for screen text)" % wref(ctx, hwnd))
    if folded:
        lines.append("(%d repeated lines folded: lighting snap %s --all)" % (folded, wref(ctx, hwnd)))
    return head + "\n" + "\n".join(lines)


def fold(lines, least=4):
    keys = [re.sub(r"^d\d+ ", "", l) if re.match(r"^d\d+ ", l) else None for l in lines]
    count = {}
    for k in keys:
        if k:
            count[k] = count.get(k, 0) + 1
    out, seen, folded = [], set(), 0
    for line, k in zip(lines, keys):
        if k and count[k] >= least:
            if k in seen:
                folded += 1
                continue
            seen.add(k)
            line = "%s (x%d, same in each item)" % (line, count[k])
        out.append(line)
    return out, folded


def cmd_windows(ctx, pos, flags):
    lines = list_windows(ctx)
    if flags.get("f"):
        filt = terms(flags["f"])
        lines = best_only([(t, l) for t, l in ((tiers(filt, l), l) for l in lines) if any(t)])
    return cap("\n".join(lines) or "no windows", "windows", lines=80)


def cmd_focus(ctx, pos, flags):
    if not pos:
        raise Fail("focus needs a window", "lighting focus w2")
    hwnd = target(ctx, pos)
    ok = pointer.front(hwnd)
    return ("ok " if ok else "err: Windows refused to switch, ") + header(ctx, hwnd)


_apps = {"t": 0.0, "list": []}
RUNNABLE = (".exe", ".bat", ".cmd", ".com", ".ps1", ".vbs", ".vbe", ".js", ".jse", ".wsf", ".msi", ".scr", ".lnk", ".url")


def start_apps(fresh=False):
    import json
    cache = D.HOME / "apps.json"
    if not fresh and not _apps["list"]:
        try:
            if time.time() - cache.stat().st_mtime < 86400:
                _apps["list"] = [tuple(a) for a in json.loads(cache.read_text("utf-8"))]
                _apps["t"] = cache.stat().st_mtime
        except (OSError, ValueError):
            pass
    if fresh or not _apps["list"]:
        import subprocess
        script = ("[Console]::OutputEncoding=[Text.Encoding]::UTF8; "
                  "Get-StartApps | Select-Object Name,AppID | ConvertTo-Json -Compress")
        res = subprocess.run(["powershell", "-NoProfile", "-NonInteractive", "-Command", script],
                             capture_output=True, timeout=30, creationflags=0x08000000)
        try:
            data = json.loads(res.stdout.decode("utf-8", "replace") or "[]")
        except ValueError:
            data = []
        data = data if isinstance(data, list) else [data]
        _apps["list"] = [(a.get("Name") or "", a.get("AppID") or "") for a in data if a.get("AppID")]
        _apps["t"] = time.time()
        try:
            cache.write_text(json.dumps(_apps["list"]), "utf-8")
        except OSError:
            pass
    return _apps["list"]


def find_app(name):
    n = name.lower()

    def rank(entry):
        t = entry[0].lower()
        return 5 if t == n else tiers([n], t)[0] + (1 if t.startswith(n) else 0)

    for fresh in (False, True):
        if fresh and time.time() - _apps["t"] < 5:
            break
        best = max(((rank(a), -len(a[0]), a) for a in start_apps(fresh)), default=None)
        if best and best[0] > 0:
            return best[2]
    return None


def run_name(name):
    if not re.fullmatch(r"[\w.-]+", name):
        return None
    exe = name if name.lower().endswith(".exe") else name + ".exe"
    root = os.environ.get("SystemRoot") or r"C:\Windows"
    if any(os.path.isfile(os.path.join(d, exe)) for d in (os.path.join(root, "System32"), root)):
        return exe
    import winreg
    for hive in (winreg.HKEY_CURRENT_USER, winreg.HKEY_LOCAL_MACHINE):
        try:
            winreg.CloseKey(winreg.OpenKey(hive, "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\" + exe))
            return exe
        except OSError:
            pass
    return None


def owner_of(wins, hint):
    words = [w for w in re.split(r"[^a-z0-9]+", (hint or "").lower()) if len(w) > 2]
    ordered = sorted(wins, key=lambda x: not x["fg"])
    for w in ordered:
        if (w["exe"] or "").lower().replace(".exe", "") in words:
            return w
    for w in ordered:
        if any(word in w["title"].lower() for word in words):
            return w
    return None


def wait_window(before, fg0, ms, hint, running):
    t0 = time.time()
    deadline = t0 + ms / 1000.0
    seen_at, pick = None, None
    while time.time() < deadline:
        wins = win.windows()
        new = [w for w in wins if w["hwnd"] not in before]
        fg = next((w for w in wins if w["fg"]), None)
        cand = next((w for w in new if w["fg"]), None) or (new[-1] if new else None)
        if not cand and fg and fg["hwnd"] != fg0:
            cand = fg
        if not cand and time.time() - t0 > (0.5 if running else 1.5):
            cand = owner_of(wins, hint)
        if cand:
            if not pick or cand["hwnd"] != pick["hwnd"]:
                pick, seen_at = cand, time.time()
            elif time.time() - seen_at > (0.3 if running else 0.6):
                return pick["hwnd"]
        time.sleep(0.15)
    return pick["hwnd"] if pick else None


def cmd_launch(ctx, pos, flags):
    if not pos:
        raise Fail("launch needs an app name, URI or path", 'lighting launch spotify | launch "spotify:search:SOS"')
    spec = " ".join(pos)
    path = spec.strip('"')
    uri = re.match(r"^[a-z][a-z0-9+.-]+:", spec, re.I) is not None
    wins0 = win.windows()
    before = {w["hwnd"] for w in wins0}
    fg0 = win.foreground()
    if os.path.exists(path):
        if path.lower().endswith(RUNNABLE) and not flags.get("yes"):
            raise Fail("%s runs a program" % os.path.basename(path), "lighting launch \"%s\" --yes (only if the user asked for it)" % path)
        os.startfile(path)
        what = hint = os.path.splitext(os.path.basename(path))[0]
    elif uri:
        scheme = spec.split(":", 1)[0].lower()
        if scheme in D.RISKY_SCHEMES and not flags.get("yes"):
            raise Fail("%s: links can run programs or open remote files" % scheme,
                       'only if the user asked for it: lighting launch "%s" --yes' % spec[:60])
        os.startfile(spec)
        what, hint = spec, spec.split(":", 1)[0]
    else:
        app = find_app(spec)
        exe = None if app and app[0].lower() == spec.lower() else run_name(spec)
        if exe:
            os.startfile(exe)
            what = hint = spec
        elif app:
            os.startfile("shell:AppsFolder\\" + app[1])
            what = hint = app[0]
        else:
            raise Fail('no app named "%s" in the start menu' % spec, "lighting launch <name as in the start menu>, a URI or a path")
    hwnd = wait_window(before, fg0, int(flags.get("timeout") or 10000), hint, owner_of(wins0, hint) is not None)
    if not hwnd:
        return "ok (started %s, no window yet) -> try: lighting windows" % what
    st = state(ctx)
    if hwnd not in before:
        if not st.launched:
            st.front0 = fg0
        st.launched[hwnd] = win.text_of(hwnd) or what
    st.hwnd = hwnd
    ctx.target = ("app", hwnd)
    return "ok (%s) -> %s" % (what, first_look(ctx, hwnd, flags))


def first_look(ctx, hwnd, flags):
    try:
        text = snapshot(ctx, hwnd, {"f": flags["f"]} if flags.get("f") else {})
    except Fail:
        return header(ctx, hwnd)
    rows = text.split("\n")
    out, chars = rows[:1], 0
    for r in rows[1:]:
        if len(out) > D.NAV_LINES or chars + len(r) > D.NAV_CHARS:
            out.append("... +%d more (lighting snap %s)" % (len(rows) - len(out), wref(ctx, hwnd)))
            break
        out.append(r)
        chars += len(r) + 1
    from lighting import keys
    out[0] += keys.hint(win.exe_of(win.pid_of(hwnd)).lower())
    return "\n".join(out)


def cleanup(ctx):
    st = state(ctx)
    wins = {h: name for h, name in st.launched.items() if win.alive(h)}
    front0, st.launched, st.front0 = st.front0, {}, None
    for h in wins:
        win.close(h)
    deadline = time.time() + 1.5
    while time.time() < deadline and any(win.alive(h) and win.visible(h) for h in wins):
        time.sleep(0.1)
    left = [h for h in wins if win.alive(h) and win.visible(h)]
    closed = [wins[h] for h in wins if h not in left]
    if st.hwnd in wins and st.hwnd not in left:
        st.hwnd = None
        if ctx.target and ctx.target[0] == "app":
            ctx.target = None
    if closed and front0 and win.alive(front0):
        win.set_foreground(front0)
    return closed, [wins[h] for h in left]


def keep(ctx, spec=None):
    st = state(ctx)
    if spec:
        return 1 if st.launched.pop(find_window(ctx, spec), None) else 0
    n, st.launched = len(st.launched), {}
    return n


def cmd_close(ctx, pos, flags):
    hwnd = find_window(ctx, pos[0])
    head = header(ctx, hwnd)
    win.close(hwnd)
    for _ in range(20):
        if not win.alive(hwnd) or not win.visible(hwnd):
            return "closed " + head
        time.sleep(0.1)
    return "asked to close " + head + " (still open, it may show a save dialog) -> try: lighting windows"


def cmd_snap(ctx, pos, flags):
    hwnd = target(ctx, pos)
    text = snapshot(ctx, hwnd, flags)
    st = state(ctx)
    if not any(flags.get(k) for k in ("f", "text", "force", "all")):
        if st.last_snap.get(hwnd) == text:
            return text.split("\n", 1)[0] + "\nunchanged since last snap (refs still valid)"
        st.last_snap[hwnd] = text
        from lighting import keys
        head, _, rest = text.partition("\n")
        text = head + keys.hint(win.exe_of(win.pid_of(hwnd)).lower()) + "\n" + rest
    return cap(text, "app-snap", lines=D.SNAP_LINES)


def before_action(ctx, hwnd):
    st = state(ctx)
    if hwnd in st.slow or not win.alive(hwnd):
        return None
    if st.full and st.full[0] == hwnd and time.time() - st.full[1] < 1.5:
        return set(st.full[2])
    t0 = time.time()
    try:
        snapshot(ctx, hwnd, {})
    except Fail:
        return None
    if time.time() - t0 > 0.5:
        st.slow.add(hwnd)
        return None
    return set(st.full[2]) if st.full and st.full[0] == hwnd else None


def after_action(ctx, hwnd, before, wait=0.6):
    if before is None:
        return ""
    st = state(ctx)
    t0, last, now, lines = time.time(), None, before, []
    while time.time() - t0 < wait:
        time.sleep(0.1)
        if not win.alive(hwnd):
            return " (window closed)"
        try:
            snapshot(ctx, hwnd, {})
        except Fail:
            return ""
        if not st.full or st.full[0] != hwnd:
            return ""
        lines = st.full[2]
        now = set(lines)
        if now != before:
            if now == last:
                break
            last = now
    if now == before:
        return ""
    added = [l for l in lines if l not in before]
    gone = len(before - now)
    if not added:
        return " (-%d gone)" % gone
    shown = added[:D.DIFF_LINES]
    s = " (+%d new%s)\n%s" % (len(added), ", -%d gone" % gone if gone else "", "\n".join(shown))
    if len(added) > len(shown):
        s += "\n... +%d more (lighting snap %s)" % (len(added) - len(shown), wref(ctx, hwnd))
    return s


def element(ctx, ref):
    st = state(ctx)
    el = st.refs.get(ref)
    if el is None:
        raise Fail("%s is unknown or from an older snapshot" % ref, "lighting snap")
    return el


def center(el):
    from lighting import uia
    try:
        r = el.CurrentBoundingRectangle
        l, t, rr, b = r.left, r.top, r.right, r.bottom
    except Exception:
        l, t, rr, b = uia.rect_of(el)
    if rr - l < 1:
        raise Fail("element has no position on screen", "lighting snap")
    return (l + rr) // 2, (t + b) // 2


def ocr_click(ctx, text_or_ref, flags):
    st = state(ctx)
    if ref_kind(text_or_ref) == "o":
        hit = st.ocr.get(text_or_ref)
        if not hit:
            raise Fail("%s is unknown" % text_or_ref, "lighting read")
    else:
        hit = None
        tl = text_or_ref.lower()
        for ref, (t, x, y) in st.ocr.items():
            if t.lower() == tl or (not hit and tl in t.lower()):
                hit = (t, x, y)
                if t.lower() == tl:
                    break
        if not hit:
            return None
    t, x, y = hit
    pointer.blitz_click(x, y, "right" if flags.get("right") else "left", 2 if flags.get("double") else 1, hwnd=st.hwnd)
    return 'ok (clicked "%s" at %d,%d via mouse, cursor restored)' % (t[:40], x, y)


def by_text(ctx, hwnd, text, roles=None):
    from lighting import uia
    tl = text.lower()
    st = state(ctx)
    for attempt in range(2):
        if attempt or not st.refs or time.time() - st.snap_t > 1.5 or getattr(st, "snap_hwnd", None) != hwnd:
            snapshot(ctx, hwnd, {})
        items = []
        for ref, cand in state(ctx).refs.items():
            role, name = uia.describe(cand)[:2]
            if not roles or role in roles:
                items.append((ref, cand, name.lower(), ROLE_RANK.get(role, 9)))
        ranked = [(4 if x[2] == tl else tiers([tl], x[2])[0], x) for x in items]
        top = max((r for r, _ in ranked), default=0)
        if top:
            best = min((x for r, x in ranked if r == top), key=lambda x: x[3])
            return best[0], best[1]
    return None, None


ROLE_RANK = {"button": 0, "link": 1, "menuitem": 2, "splitbutton": 2, "checkbox": 3, "radio": 3, "tab": 3,
             "edit": 4, "combobox": 4, "treeitem": 5, "listitem": 6, "dataitem": 6}


def note_target(ctx, el):
    from lighting import uia
    try:
        role, name = uia.describe(el)[:2]
        ctx.last_target = {"name": name, "role": role}
    except Exception:
        ctx.last_target = None


def cmd_click(ctx, pos, flags):
    from lighting import uia
    if not pos:
        raise Fail("click needs a ref or \"text\"")
    arg = pos[0]
    kind = ref_kind(arg)
    if kind == "o":
        return ocr_click(ctx, arg, flags)
    if kind == "w" or arg.lower().startswith("app:"):
        return cmd_focus(ctx, pos, flags)
    hwnd = target(ctx)
    if kind == "d":
        el = element(ctx, arg)
    else:
        ref, el = by_text(ctx, hwnd, arg)
        if el is None:
            res = ocr_click(ctx, pos[0], flags) if state(ctx).ocr else None
            if res:
                return res
            raise Fail('nothing called "%s" in %s' % (pos[0], wref(ctx, hwnd)), "lighting snap, or lighting read for screen text")
        arg = ref
    note_target(ctx, el)
    x, y = center(el)
    before = before_action(ctx, hwnd)
    if flags.get("mouse") or flags.get("right") or flags.get("double"):
        pointer.blitz_click(x, y, "right" if flags.get("right") else "left", 2 if flags.get("double") else 1, hwnd=hwnd)
        return "ok %s (mouse click, cursor restored)%s" % (arg, after_action(ctx, hwnd, before))
    pointer.show(x, y)
    how = uia.act(el)
    if how:
        if before is None:
            time.sleep(0.15)
        return "ok %s (%s in background)%s" % (arg, how, after_action(ctx, hwnd, before))
    pointer.blitz_click(x, y, hwnd=hwnd)
    return "ok %s (mouse click, cursor restored)%s" % (arg, after_action(ctx, hwnd, before))


FIELD_ROLES = ("edit", "combobox", "document", "spinner")


def focused_in(hwnd):
    from lighting import uia
    for attempt in (1, 2):
        try:
            el = uia.api()[0].GetFocusedElement()
        except Exception:
            el = None
        if el is not None and (el.CurrentProcessId == win.pid_of(hwnd) or win.foreground() == hwnd):
            return el
        if attempt == 1:
            if not pointer.wait_idle():
                raise Fail("you are using mouse or keyboard right now", "retry in a moment")
            if pointer.front(hwnd):
                time.sleep(0.15)
    raise Fail("nothing focused in %s" % win.text_of(hwnd)[:40], "lighting click the field first, or type d<N> text")


def cmd_type(ctx, pos, flags):
    if not pos:
        raise Fail("type needs a d-ref, a field name or focused", 'lighting type d5 "hello" | type "Search" hello')
    if ref_kind(pos[0]) == "d":
        el = element(ctx, pos[0])
    elif pos[0].lower() == "focused":
        el = focused_in(target(ctx))
    else:
        hwnd = target(ctx)
        ref, el = by_text(ctx, hwnd, pos[0], FIELD_ROLES)
        if el is None:
            ref, el = by_text(ctx, hwnd, pos[0])
        if el is None:
            raise Fail('no field called "%s" in %s' % (pos[0], wref(ctx, hwnd)), "lighting snap")
        pos = [ref] + list(pos[1:])
    note_target(ctx, el)
    text = ctx.secret if ctx.secret is not None else " ".join(pos[1:])
    hwnd = target(ctx)
    before = before_action(ctx, hwnd)
    return put_text(ctx, hwnd, el, pos[0], text, flags) + after_action(ctx, hwnd, before)


def settle_ui(hwnd, least, most):
    from lighting import uia
    time.sleep(least)
    t0, last = time.time(), None
    while time.time() - t0 < most - least:
        try:
            n = len(uia.collect(hwnd)[1])
        except Exception:
            return
        if n == last:
            return
        last = n
        time.sleep(0.08)


def put_text(ctx, hwnd, el, label, text, flags):
    from lighting import uia
    try:
        pointer.show(*center(el))
    except Fail:
        pass
    shown = "%d %schars" % (len(text), "secret " if ctx.secret is not None else "")
    keys = [k for k in ("tab", "enter") if flags.get(k)]
    rich = win.class_of(hwnd) == CHROMIUM_CLASS
    if not keys and not rich:
        full = (uia.get_value(el) or "") + text if flags.get("append") else text
        if uia.set_value(el, full):
            return "ok %s (typed %s in background)" % (label, shown)
    saved = win.clip_get()

    terminal = win.class_of(hwnd) in TERMINALS

    def paste():
        try:
            el.SetFocus()
        except Exception:
            pass
        if not flags.get("append") and not terminal:
            win.press("ctrl+a")
        win.clip_set(text)
        win.press("ctrl+v")
        time.sleep(0.08)
        for k in keys:
            settle_ui(hwnd, 0.35, 1.0)
            win.press(k)

    pointer.with_focus(hwnd, paste, stay=bool(flags.get("stay")))
    if saved is not None:
        win.clip_set(saved)
    return "ok %s (pasted %s%s, window was briefly in front)" % (label, shown, "".join(" +" + k for k in keys))


def cmd_press(ctx, pos, flags):
    if not pos:
        raise Fail("press needs keys", "lighting press ctrl+s")
    if all(p.lower() in win.MEDIA for p in pos):
        for combo in pos:
            win.press(combo)
            time.sleep(0.03)
        return "ok (%s, media key for the whole system)" % " ".join(pos)
    hwnd = target(ctx)
    game = bool(flags.get("game"))
    before = None if game else before_action(ctx, hwnd)

    def go():
        for combo in pos:
            win.press(combo, scancode=game)
            time.sleep(0.03)

    try:
        pointer.with_focus(hwnd, go, stay=bool(flags.get("stay")))
    except ValueError as e:
        raise Fail(str(e))
    diff = after_action(ctx, hwnd, before)
    if diff.startswith(" (+") and len(pos) == 1:
        from lighting import keys
        keys.learn(win.exe_of(win.pid_of(hwnd)).lower(), pos[0], diff.split("\n", 2)[1])
    return "ok (%s sent to %s)%s" % (" ".join(pos), wref(ctx, hwnd), diff)


def cmd_scroll(ctx, pos, flags):
    from lighting import uia
    down = not any(p.lower() == "up" for p in pos)
    refs = [p for p in pos if ref_kind(p) == "d"]
    hwnd = target(ctx)
    if refs:
        el = element(ctx, refs[0])
        if uia.scroll(el, down):
            return "ok (scrolled %s in background)" % refs[0]
        x, y = center(el)
    else:
        l, t, r, b = win.rect(hwnd)
        x, y = (l + r) // 2, (t + b) // 2
    pointer.show(x, y)

    def wheel():
        saved = win.cursor()
        win.set_cursor(x, y)
        win.wheel(-360 if down else 360)
        win.set_cursor(*saved)

    pointer.with_focus(hwnd, wheel)
    return "ok (wheel %s, cursor restored)" % ("down" if down else "up")


def cmd_drag(ctx, pos, flags):
    if len(pos) < 2:
        raise Fail("drag needs two d-refs", "lighting drag d3 d9")
    a, b = center(element(ctx, pos[0])), center(element(ctx, pos[1]))
    pointer.blitz_drag(a, b, hwnd=target(ctx))
    return "ok (dragged %s to %s, cursor restored)" % (pos[0], pos[1])


def cmd_hover(ctx, pos, flags):
    raise Fail("hover on desktop apps would steal your mouse", "use lighting shot or read to look, or click")


def region(ctx, pos):
    if pos and pos[0] == "screen":
        return None, win.virtual_screen()
    hwnd = target(ctx, pos)
    if win.user32.IsIconic(hwnd):
        raise Fail("%s is minimized, nothing to capture" % wref(ctx, hwnd), "lighting focus %s" % wref(ctx, hwnd))
    return hwnd, win.rect(hwnd)


def grab(box, hwnd=None):
    from PIL import ImageGrab
    if hwnd and hwnd != win.foreground():
        img = win.print_window(hwnd)
        if img is not None:
            return img
    pointer.hide()
    return ImageGrab.grab(bbox=box, all_screens=True)


def cmd_read(ctx, pos, flags):
    from lighting import ocr
    hwnd, box = region(ctx, pos)
    img = grab(box, hwnd)
    lines = ocr.recognize(img, flags.get("lang") or ctx.cfg.get("ocr_lang"))
    st = state(ctx)
    st.ocr = {}
    out = []
    filt = terms(flags.get("f"))
    for i, (text, x, y, w, h) in enumerate(lines, 1):
        cx, cy = int(box[0] + x + w / 2), int(box[1] + y + h / 2)
        ref = "o%d" % i
        st.ocr[ref] = (text, cx, cy)
        t = tiers(filt, text) if filt else None
        if t is not None and not any(t):
            continue
        out.append((t, '%s "%s" @%d,%d' % (ref, text.replace('"', "'"), cx, cy)))
    out = best_only(out)
    head = header(ctx, hwnd, " (screen text, %d lines)" % len(lines)) if hwnd else "[screen] (screen text, %d lines)" % len(lines)
    return cap(head + "\n" + ("\n".join(out) or "(no text found)"), "read", lines=D.SNAP_LINES)


def cmd_shot(ctx, pos, flags):
    from lighting.browser import save_image
    hwnd, box = region(ctx, pos)
    img = grab(box, hwnd)
    key = "app-%s" % (hwnd or "screen")
    return save_image(ctx, img, key, bool(flags.get("if-changed")),
                      int(flags.get("width") or ctx.cfg.get("shot_width") or D.SHOT_WIDTH))


def cmd_clip(ctx, pos, flags):
    if pos and pos[0] == "set":
        win.clip_set(ctx.secret if ctx.secret is not None else " ".join(pos[1:]))
        return "ok (clipboard set)"
    text = win.clip_get()
    if text is None:
        raise Fail("clipboard is busy")
    return cap(text or "(clipboard is empty)", "clip", chars=D.TEXT_CHARS)

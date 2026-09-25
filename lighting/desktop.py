import os
import re
import time

from lighting import defaults as D
from lighting import pointer
from lighting import win
from lighting.common import Fail, cap, hit, ref_kind, terms

BROWSER_EXES = {"brave.exe", "chrome.exe", "msedge.exe", "opera.exe", "vivaldi.exe"}
CHROMIUM_CLASS = "Chrome_WidgetWin_1"


class AppState:
    def __init__(self):
        self.windows = {}
        self.refs = {}
        self.ids = {}
        self.seq = 0
        self.ocr = {}
        self.hwnd = None


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
    filt = terms(flags.get("f"))
    lines, refs, row = [], {}, None
    for el in items:
        role, name, extra = uia.describe(el)
        name, extra = BIDI.sub("", name), BIDI.sub("", extra)
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
        if filt and not hit(filt, name, role, extra):
            continue
        if role == "text":
            lines.append("  %s" % name)
            continue
        label = name if len(name) <= 60 else name[:57] + "..."
        lines.append("%s %s%s%s" % (ref, role, ' "%s"' % label.replace('"', "'") if label else "", extra))
        if role in ("listitem", "dataitem"):
            row = (len(lines) - 1, uia.rect_of(el), name)
    st.refs = refs
    head = header(ctx, hwnd, " (%d controls%s)" % (len(refs), ", browser page hidden: use lighting snap for the page" if skip else ""))
    if not lines:
        lines.append("(no controls found: try lighting read %s for screen text)" % wref(ctx, hwnd))
    return head + "\n" + "\n".join(lines)


def cmd_windows(ctx, pos, flags):
    lines = list_windows(ctx)
    if flags.get("f"):
        lines = [l for l in lines if hit(terms(flags["f"]), l)]
    return cap("\n".join(lines) or "no windows", "windows", lines=80)


def cmd_focus(ctx, pos, flags):
    if not pos:
        raise Fail("focus needs a window", "lighting focus w2")
    hwnd = target(ctx, pos)
    ok = pointer.front(hwnd)
    return ("ok " if ok else "err: Windows refused to switch, ") + header(ctx, hwnd)


def cmd_snap(ctx, pos, flags):
    hwnd = target(ctx, pos)
    return cap(snapshot(ctx, hwnd, flags), "app-snap", lines=D.SNAP_LINES)


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
        el = None
        tl = arg.lower()
        for attempt in range(2):
            if attempt or not state(ctx).refs:
                snapshot(ctx, hwnd, {})
            items = [(ref, cand, uia.describe(cand)[1].lower()) for ref, cand in state(ctx).refs.items()]
            hit = next((x for x in items if x[2] == tl), None) or next((x for x in items if tl in x[2]), None)
            if hit:
                arg, el = hit[0], hit[1]
                break
        if el is None:
            res = ocr_click(ctx, pos[0], flags) if state(ctx).ocr else None
            if res:
                return res
            raise Fail('nothing called "%s" in %s' % (pos[0], wref(ctx, hwnd)), "lighting snap, or lighting read for screen text")
    x, y = center(el)
    if flags.get("mouse") or flags.get("right") or flags.get("double"):
        pointer.blitz_click(x, y, "right" if flags.get("right") else "left", 2 if flags.get("double") else 1, hwnd=hwnd)
        return "ok %s (mouse click, cursor restored)" % arg
    pointer.show(x, y)
    how = uia.act(el)
    if how:
        time.sleep(0.15)
        return "ok %s (%s in background)" % (arg, how)
    pointer.blitz_click(x, y, hwnd=hwnd)
    return "ok %s (mouse click, cursor restored)" % arg


def cmd_type(ctx, pos, flags):
    from lighting import uia
    if not pos or ref_kind(pos[0]) != "d":
        raise Fail("type needs a d-ref", 'lighting type d5 "hello"')
    el = element(ctx, pos[0])
    text = ctx.secret if ctx.secret is not None else " ".join(pos[1:])
    if flags.get("append"):
        cur = uia.get_value(el) or ""
        text = cur + text
    x, y = center(el)
    pointer.show(x, y)
    shown = "%d %schars" % (len(text), "secret " if ctx.secret is not None else "")
    if uia.set_value(el, text):
        return "ok %s (typed %s in background)" % (pos[0], shown)
    hwnd = target(ctx)
    saved = win.clip_get()

    def paste():
        try:
            el.SetFocus()
        except Exception:
            pass
        if not flags.get("append"):
            win.press("ctrl+a")
        win.clip_set(text)
        win.press("ctrl+v")
        time.sleep(0.08)

    pointer.with_focus(hwnd, paste, stay=bool(flags.get("stay")))
    if saved is not None:
        win.clip_set(saved)
    return "ok %s (pasted %s, window was briefly in front)" % (pos[0], shown)


def cmd_press(ctx, pos, flags):
    if not pos:
        raise Fail("press needs keys", "lighting press ctrl+s")
    hwnd = target(ctx)
    game = bool(flags.get("game"))

    def go():
        for combo in pos:
            win.press(combo, scancode=game)
            time.sleep(0.03)

    try:
        pointer.with_focus(hwnd, go, stay=bool(flags.get("stay")))
    except ValueError as e:
        raise Fail(str(e))
    return "ok (%s sent to %s)" % (" ".join(pos), wref(ctx, hwnd))


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
        if filt and not hit(filt, text):
            continue
        out.append('%s "%s" @%d,%d' % (ref, text.replace('"', "'"), cx, cy))
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

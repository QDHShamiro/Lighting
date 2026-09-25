import os
import subprocess
import time

from lighting import defaults as D
from lighting import install
from lighting import pointer
from lighting import uia
from lighting import win
from lighting.common import Fail

PAGES = {"brave": "brave://extensions/", "chrome": "chrome://extensions/", "edge": "edge://extensions/"}
TITLES = ("erweiterungen", "extensions", "extensiones", "extensões")
DEV = ("devMode", ("entwicklermodus", "developer mode"))
LOAD = ("loadUnpacked", ("entpackte erweiterung laden", "load unpacked"))
OK_NAMES = ("ordner auswählen", "select folder", "auswählen", "select")


def wait(fn, seconds, step=0.25):
    end = time.time() + seconds
    while time.time() < end:
        try:
            value = fn()
        except Exception:
            value = None
        if value:
            return value
        time.sleep(step)
    return None


def match(hwnd, auto_id, names, roles=None):
    U = uia.api()[1]
    _, items = uia.collect(hwnd)
    for el in items:
        aid = uia.cached(el, U.UIA_AutomationIdPropertyId) or ""
        name = (uia.cached(el, U.UIA_NamePropertyId) or "").lower()
        ctype = uia.cached(el, U.UIA_ControlTypePropertyId)
        if roles and ctype not in roles:
            continue
        if aid == auto_id or any(n in name for n in names):
            return el
    return None


def toggle_near(hwnd, names):
    U = uia.api()[1]
    _, items = uia.collect(hwnd, with_text=True)
    labels, toggles = [], []
    for el in items:
        name = (uia.cached(el, U.UIA_NamePropertyId) or "").lower()
        aid = uia.cached(el, U.UIA_AutomationIdPropertyId) or ""
        if aid == "devMode" and uia.cached(el, U.UIA_IsTogglePatternAvailablePropertyId):
            return el
        if uia.cached(el, U.UIA_IsTogglePatternAvailablePropertyId):
            if any(n in name for n in names):
                return el
            toggles.append(el)
        elif any(n == name.strip() or n in name for n in names):
            labels.append(el)
    best, dist = None, 10 ** 9
    for lab in labels:
        l, t, r, b = uia.rect_of(lab)
        lx, ly = (l + r) / 2, (t + b) / 2
        for tg in toggles:
            tl, tt, tr, tb = uia.rect_of(tg)
            tx, ty = (tl + tr) / 2, (tt + tb) / 2
            d = abs(tx - lx) + 3 * abs(ty - ly)
            if d < dist and abs(ty - ly) < 60:
                best, dist = tg, d
    return best


def is_on(el):
    U = uia.api()[1]
    p = uia.pattern(el, U.UIA_TogglePatternId, U.IUIAutomationTogglePattern)
    if p is not None:
        return p.CurrentToggleState == 1
    name = (uia.cached(el, U.UIA_NamePropertyId) or "").lower()
    return None if not name else None


def center(el):
    r = el.CurrentBoundingRectangle
    return (r.left + r.right) // 2, (r.top + r.bottom) // 2


def host_for(ctx, brand):
    for h in ctx.d.hosts:
        if h.alive and brand in h.brand:
            return h
    return None


def browser_window(exe_name, titles=None):
    for w in win.windows():
        if (w["exe"] or "").lower() != exe_name:
            continue
        if titles and not any(t in w["title"].lower() for t in titles):
            continue
        return w["hwnd"]
    return None


NEW_TAB = ("neuer tab", "new tab", "nueva pestaña", "nouvel onglet")


def is_new_tab(hwnd):
    return any(win.text_of(hwnd).lower().startswith(t) for t in NEW_TAB)


def open_page(hwnd, url):
    def go():
        time.sleep(0.15)
        if not is_new_tab(hwnd):
            for _ in range(2):
                win.press("ctrl+t")
                if wait(lambda: is_new_tab(hwnd), 2.5, 0.1):
                    break
                pointer.front(hwnd)
                time.sleep(0.2)
        if not is_new_tab(hwnd):
            raise Fail("could not open an empty tab, so nothing was typed (keyboard input blocked?)",
                       "click the browser window once, then run lighting setup again")
        win.press("ctrl+l")
        time.sleep(0.15)
        if not is_new_tab(hwnd):
            raise Fail("the empty tab lost focus, nothing was typed", "run lighting setup again")
        win.type_unicode(url)
        time.sleep(0.08)
        win.press("enter")

    pointer.with_focus(hwnd, go, stay=True)


def run(ctx, name, log):
    exe = install.browser_exe(name)
    if not exe:
        raise Fail("%s is not installed" % name)
    exe_name = os.path.basename(exe).lower()
    if not browser_window(exe_name):
        subprocess.Popen([exe], creationflags=0x00000008 | 0x00000200, close_fds=True)
        host = wait(lambda: host_for(ctx, name), 8)
        if host:
            log("%s started, the extension was already installed" % name)
            return host
    hwnd = wait(lambda: browser_window(exe_name), 15)
    if not hwnd:
        raise Fail("no %s window appeared" % name)
    open_page(hwnd, PAGES.get(name, "chrome://extensions/"))
    hwnd = wait(lambda: browser_window(exe_name, TITLES), 12)
    if not hwnd:
        raise Fail("the extensions page did not open in %s" % name)
    pointer.front(hwnd)
    dev = wait(lambda: toggle_near(hwnd, DEV[1]), 12, 0.4)
    if dev is None:
        raise Fail("developer mode switch not found on the extensions page")
    if not is_on(dev):
        uia.act(dev)
        log("developer mode switched on")
        time.sleep(0.8)
    load = wait(lambda: match(hwnd, *LOAD), 8, 0.4)
    if load is None:
        raise Fail("'Load unpacked' button not found")
    before = {w["hwnd"] for w in win.windows(owned=True)}

    def dialog():
        for w in win.windows(owned=True):
            if w["hwnd"] not in before and w["cls"] == "#32770":
                return w["hwnd"]
        return None

    uia.act(load)
    dlg = wait(dialog, 5)
    if not dlg:
        x, y = center(load)
        pointer.blitz_click(x, y, hwnd=hwnd)
        dlg = wait(dialog, 6)
    if not dlg:
        raise Fail("the folder dialog did not open")
    log("folder dialog open, selecting %s" % D.EXT)
    U = uia.api()[1]
    _, items = uia.collect(dlg)
    edits = [e for e in items if uia.cached(e, U.UIA_ControlTypePropertyId) == 50004]
    edit = next((e for e in edits if (uia.cached(e, U.UIA_AutomationIdPropertyId) or "") in ("1152", "1148")), None)
    edit = edit or next((e for e in edits if any(k in (uia.cached(e, U.UIA_NamePropertyId) or "").lower() for k in ("ordner", "folder"))), None)
    edit = edit or (edits[-1] if edits else None)
    if edit is None or not uia.set_value(edit, str(D.EXT)):
        raise Fail("could not type the folder path into the dialog")
    time.sleep(0.2)
    buttons = [e for e in items if uia.cached(e, U.UIA_ControlTypePropertyId) == 50000]
    ok = next((b for b in buttons if (uia.cached(b, U.UIA_AutomationIdPropertyId) or "") == "1"), None)
    ok = ok or next((b for b in buttons if (uia.cached(b, U.UIA_NamePropertyId) or "").lower() in OK_NAMES), None)
    if ok is None:
        raise Fail("the dialog's select button was not found")
    uia.act(ok)
    host = wait(lambda: host_for(ctx, name), 20, 0.3)
    if not host:
        raise Fail("extension loaded, but it did not connect (check brave://extensions for errors)")
    try:
        host.call("close-url", {"prefix": PAGES.get(name, "")}, timeout=5)
    except Exception:
        pass
    log("extension loaded and connected")
    return host


def cmd_autoload(ctx, pos, flags):
    from lighting import browser
    wanted = pos[0] if pos else (ctx.cfg.get("browser") if ctx.cfg.get("browser") in D.BROWSER_EXES else None)
    names = [wanted] if wanted else install.installed_browsers()[:1]
    lines = []
    for name in names:
        if host_for(ctx, name):
            lines.append("%s already connected" % name)
            break
        try:
            run(ctx, name, lambda m: lines.append(m))
            browser.push_config(ctx)
            break
        except Fail as e:
            lines.append("%s: %s" % (name, e))
    return "\n".join(lines) or "no supported browser found"

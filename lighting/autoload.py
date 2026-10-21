import os
import time

from lighting import browser
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
    _, items = uia.collect(hwnd)
    for el in items:
        aid = uia.auto_id(el)
        role, name, _ = uia.describe(el)
        name = name.lower()
        if roles and role not in roles:
            continue
        if aid == auto_id or any(n in name for n in names):
            return el
    return None


def toggle_near(hwnd, names):
    _, items = uia.collect(hwnd, with_text=True)
    labels, toggles = [], []
    for el in items:
        name = uia.describe(el)[1].lower()
        aid = uia.auto_id(el)
        if aid == "devMode" and uia.toggleable(el):
            return el
        if uia.toggleable(el):
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
    return uia.toggled(el)


def center(el):
    l, t, r, b = uia.live_rect(el)
    return (l + r) // 2, (t + b) // 2


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
                win.press(win.MOD + "+t")
                if wait(lambda: is_new_tab(hwnd), 2.5, 0.1):
                    break
                pointer.front(hwnd)
                time.sleep(0.2)
        if not is_new_tab(hwnd):
            raise Fail("could not open an empty tab, so nothing was typed (keyboard input blocked?)",
                       "click the browser window once, then run lighting setup again")
        win.press(win.MOD + "+l")
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
    exe_name = D.BROWSER_PROCS.get(name) or os.path.basename(exe).lower()
    if not browser_window(exe_name):
        browser.start_browser(exe)
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
        if not D.WIN:
            return win.file_dialog(hwnd, before)
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
    if D.WIN:
        pick_windows(dlg)
    else:
        pointer.with_focus(hwnd, lambda: win.choose_folder(dlg, str(D.EXT)), stay=True)
    host = wait(lambda: host_for(ctx, name), 20, 0.3)
    if not host:
        raise Fail("extension loaded, but it did not connect (check brave://extensions for errors)")
    try:
        host.call("close-url", {"prefix": PAGES.get(name, "")}, timeout=5)
    except Exception:
        pass
    log("extension loaded and connected")
    return host


def pick_windows(dlg):
    _, items = uia.collect(dlg)
    edits = [e for e in items if uia.describe(e)[0] == "edit"]
    edit = next((e for e in edits if uia.auto_id(e) in ("1152", "1148")), None)
    edit = edit or next((e for e in edits if any(k in uia.describe(e)[1].lower() for k in ("ordner", "folder"))), None)
    edit = edit or (edits[-1] if edits else None)
    if edit is None or not uia.set_value(edit, str(D.EXT)):
        raise Fail("could not type the folder path into the dialog")
    time.sleep(0.2)
    buttons = [e for e in items if uia.describe(e)[0] == "button"]
    ok = next((b for b in buttons if uia.auto_id(b) == "1"), None)
    ok = ok or next((b for b in buttons if uia.describe(b)[1].lower() in OK_NAMES), None)
    if ok is None:
        raise Fail("the dialog's select button was not found")
    uia.act(ok)


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

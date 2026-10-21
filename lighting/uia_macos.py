import time

import ApplicationServices as AS

from lighting import defaults as D
from lighting import win_macos as W

_state = {}
ATTRS = ["AXRole", "AXSubrole", "AXTitle", "AXDescription", "AXValue", "AXEnabled", "AXPosition", "AXSize",
         "AXIdentifier", "AXChildren", "AXFocused", "AXSelected", "AXExpanded", "AXPlaceholderValue", "AXHelp",
         "AXMenuItemCmdChar", "AXMenuItemCmdModifiers", "AXDOMIdentifier"]
ROLE = {"AXButton": "button", "AXCheckBox": "checkbox", "AXPopUpButton": "combobox", "AXComboBox": "combobox",
        "AXTextField": "edit", "AXTextArea": "edit", "AXSearchField": "edit", "AXSecureTextField": "edit",
        "AXLink": "link", "AXRow": "listitem", "AXCell": "dataitem", "AXMenuItem": "menuitem",
        "AXMenuBarItem": "menuitem", "AXMenuButton": "splitbutton", "AXRadioButton": "radio", "AXSlider": "slider",
        "AXIncrementor": "spinner", "AXOutlineRow": "treeitem", "AXWebArea": "document", "AXDisclosureTriangle": "button",
        "AXTab": "tab", "AXColorWell": "button", "AXDockItem": "button", "AXHeading": "header"}
SUBROLE = {"AXSwitch": "checkbox", "AXToggle": "checkbox", "AXTabButton": "tab", "AXSearchField": "edit",
           "AXSecureTextField": "edit", "AXOutlineRow": "treeitem", "AXTableRow": "listitem"}
CTYPE = {"button": 50000, "checkbox": 50002, "combobox": 50003, "edit": 50004, "link": 50005, "image": 50006,
         "listitem": 50007, "list": 50008, "menuitem": 50011, "radio": 50013, "slider": 50015, "spinner": 50016,
         "tab": 50019, "text": 50020, "treeitem": 50024, "dataitem": 50029, "document": 50030, "splitbutton": 50031,
         "header": 50035, "control": 50025}
CONTAINERS = {"AXList": "list", "AXTable": "list", "AXOutline": "list", "AXImage": "image", "AXGroup": "control"}
MODS = {0: "cmd", 1: "shift+cmd", 2: "alt+cmd", 3: "alt+shift+cmd", 4: "ctrl+cmd", 8: "", 12: "ctrl"}


class El:
    __slots__ = ("ref", "a", "pid")

    def __init__(self, ref, values, pid):
        self.ref = ref
        self.a = values
        self.pid = pid


def api():
    return None, None


def fetch(ref, pid):
    try:
        err, vals = AS.AXUIElementCopyMultipleAttributeValues(ref, ATTRS, 0, None)
    except Exception:
        return None
    if err != 0 or vals is None:
        return None
    out = {}
    for k, v in zip(ATTRS, vals):
        if v is None:
            continue
        try:
            if AS.AXValueGetType(v) == AS.kAXValueAXErrorType:
                continue
        except Exception:
            pass
        out[k] = v
    return El(ref, out, pid)


def role_of(el):
    r, s = str(el.a.get("AXRole") or ""), str(el.a.get("AXSubrole") or "")
    if s in SUBROLE:
        return SUBROLE[s]
    if r == "AXStaticText":
        return "text"
    return ROLE.get(r) or CONTAINERS.get(r) or "control"


def rect_of(el):
    p, s = W.point_of(el.a.get("AXPosition")), W.size_of(el.a.get("AXSize"))
    if not p or not s:
        return 0, 0, 0, 0
    return int(p[0]), int(p[1]), int(p[0] + s[0]), int(p[1] + s[1])


def text(v):
    if v is None:
        return ""
    if isinstance(v, (int, float)) and not isinstance(v, bool):
        return ""
    try:
        return str(v)
    except Exception:
        return ""


def name_of(el):
    r = role_of(el)
    for key in ("AXTitle", "AXDescription"):
        t = text(el.a.get(key)).strip()
        if t:
            return t
    if r == "text":
        return text(el.a.get("AXValue")).strip()
    for key in ("AXPlaceholderValue", "AXHelp"):
        t = text(el.a.get(key)).strip()
        if t:
            return t
    if r in ("listitem", "dataitem", "treeitem"):
        return inner_text(el, 3)
    return ""


def inner_text(el, depth):
    if depth <= 0:
        return ""
    parts = []
    for kid in children(el):
        t = text(kid.a.get("AXValue")).strip() if role_of(kid) in ("text", "edit") else ""
        t = t or text(kid.a.get("AXTitle")).strip() or inner_text(kid, depth - 1)
        if t:
            parts.append(t)
    return " ".join(parts)[:200]


def root_of(hwnd):
    ref = W.ax_of(hwnd)
    return fetch(ref, W.pid_of(hwnd)) if ref is not None else None


def children(el):
    out = []
    for ref in el.a.get("AXChildren") or []:
        kid = fetch(ref, el.pid)
        if kid is not None:
            out.append(kid)
    return out


def walk(root, budget_ms=None, limit=4000):
    budget_ms = budget_ms or D.UIA_BUDGET_MS
    t0 = time.perf_counter()
    queue, out = [root], []
    while queue and len(out) < limit:
        el = queue.pop(0)
        out.append(el)
        queue[0:0] = children(el)
        if (time.perf_counter() - t0) * 1000 > budget_ms:
            break
    return out[1:]


def collect(hwnd, with_text=False, skip_rect=None):
    root = root_of(hwnd)
    if root is None:
        return None, []
    wl, wt, wr, wb = W.rect(hwnd)
    items = []
    for el in walk(root):
        role = role_of(el)
        if role in ("control", "list", "image") or (role == "text" and not with_text):
            continue
        l, t, r, b = rect_of(el)
        if r - l < 2 or b - t < 2 or r < wl or l > wr or b < wt or t > wb:
            continue
        if skip_rect and l >= skip_rect[0] and t >= skip_rect[1] and r <= skip_rect[2] and b <= skip_rect[3]:
            continue
        items.append(el)
    return root, items


def describe(el):
    role = role_of(el)
    name = " ".join(name_of(el).split())
    s = ""
    val = el.a.get("AXValue")
    if role == "checkbox" and isinstance(val, (int, float)):
        s += " [x]" if val == 1 else " [-]" if val == 2 else " [ ]"
    if role in ("edit", "combobox", "spinner", "slider"):
        if str(el.a.get("AXSubrole") or "") == "AXSecureTextField":
            s += " =***"
        else:
            v = text(val) if not isinstance(val, (int, float)) or role in ("slider", "spinner") else str(val)
            v = " ".join(v.split())
            if v and v != name:
                s += ' ="%s"' % (v[:40] + "..." if len(v) > 43 else v).replace('"', "'")
    if el.a.get("AXSelected") is True and role in ("listitem", "dataitem", "tab", "treeitem", "radio"):
        s += " *"
    if role == "radio" and val == 1:
        s += " *"
    if "AXExpanded" in el.a and role in ("treeitem", "combobox", "button", "menuitem"):
        s += " (expanded)" if el.a.get("AXExpanded") else " (collapsed)"
    if el.a.get("AXEnabled") is False:
        s += " (disabled)"
    return role, name, s


def cached(el, key):
    return el.a.get(key)


def runtime_id(el):
    try:
        import CoreFoundation
        return (el.pid, int(CoreFoundation.CFHash(el.ref)), rect_of(el))
    except Exception:
        return None


def actions(el):
    try:
        err, names = AS.AXUIElementCopyActionNames(el.ref, None)
    except Exception:
        return []
    return [str(n) for n in names or []] if err == 0 else []


def act(el, how=None):
    role = role_of(el)
    acts = actions(el)
    if role == "edit":
        if W.ax_set(el.ref, "AXFocused", True):
            return "focused"
    for name, label in (("AXPress", "pressed"), ("AXPick", "picked"), ("AXConfirm", "confirmed"),
                        ("AXShowMenu", "opened its menu")):
        if name in acts and W.ax_do(el.ref, name):
            return label
    if role in ("listitem", "dataitem", "treeitem", "tab") and W.ax_set(el.ref, "AXSelected", True):
        return "selected"
    return None


def settable(el, attr):
    try:
        err, ok = AS.AXUIElementIsAttributeSettable(el.ref, attr, None)
    except Exception:
        return False
    return err == 0 and bool(ok)


def set_value(el, value):
    if not settable(el, "AXValue"):
        return False
    return W.ax_set(el.ref, "AXValue", value)


def get_value(el):
    v = W.ax_get(el.ref, "AXValue")
    return text(v) if v is not None else None


def main_scroller(hwnd):
    root = root_of(hwnd)
    if root is None:
        return None
    best, area = None, 0
    for el in walk(root, 400):
        if str(el.a.get("AXRole") or "") != "AXScrollArea":
            continue
        l, t, r, b = rect_of(el)
        if (r - l) * (b - t) > area:
            best, area = el, (r - l) * (b - t)
    return best


def scroll(el, down=True, large=True):
    bar = W.ax_get(el.ref, "AXVerticalScrollBar")
    if bar is None:
        for ref in el.a.get("AXChildren") or []:
            if str(W.ax_get(ref, "AXOrientation") or "") == "AXVerticalOrientation":
                bar = ref
                break
    if bar is None:
        return False
    for action in (("AXIncrementPage" if large else "AXIncrement") if down else ("AXDecrementPage" if large else "AXDecrement"),):
        if W.ax_do(bar, action):
            return True
    v = W.ax_get(bar, "AXValue")
    if not isinstance(v, (int, float)):
        return False
    step = 0.25 if large else 0.08
    return W.ax_set(bar, "AXValue", max(0.0, min(1.0, float(v) + (step if down else -step))))


def element_at(x, y):
    wide = AS.AXUIElementCreateSystemWide()
    try:
        err, ref = AS.AXUIElementCopyElementAtPosition(wide, float(x), float(y), None)
    except Exception:
        return None
    if err != 0 or ref is None:
        return None
    err, pid = AS.AXUIElementGetPid(ref, None)
    return fetch(ref, pid if err == 0 else 0)


def page_rect(hwnd):
    root = root_of(hwnd)
    if root is None:
        return None
    for el in walk(root, 300, 600):
        if str(el.a.get("AXRole") or "") == "AXWebArea":
            return rect_of(el)
    return None


def shortcut(el):
    ch = text(el.a.get("AXMenuItemCmdChar")).strip()
    if not ch:
        return None
    mods = el.a.get("AXMenuItemCmdModifiers")
    pre = MODS.get(int(mods) if isinstance(mods, (int, float)) else 0, "cmd")
    return (pre + "+" if pre else "") + ch.lower()


def texts(el):
    return [t for t in (name_of(el), text(el.a.get("AXValue"))) if t]


def auto_id(el):
    return text(el.a.get("AXIdentifier") or el.a.get("AXDOMIdentifier"))


def toggleable(el):
    return role_of(el) == "checkbox"


def toggled(el):
    v = W.ax_get(el.ref, "AXValue")
    return None if not isinstance(v, (int, float)) else v == 1


def live_rect(el):
    p, s = W.point_of(W.ax_get(el.ref, "AXPosition")), W.size_of(W.ax_get(el.ref, "AXSize"))
    if not p or not s:
        return rect_of(el)
    return int(p[0]), int(p[1]), int(p[0] + s[0]), int(p[1] + s[1])


def focused():
    wide = AS.AXUIElementCreateSystemWide()
    ref = W.ax_get(wide, "AXFocusedUIElement")
    if ref is None:
        return None
    err, pid = AS.AXUIElementGetPid(ref, None)
    return fetch(ref, pid if err == 0 else 0)


def pid_of(el):
    return el.pid if el is not None else 0


def focus(el):
    return W.ax_set(el.ref, "AXFocused", True)


def focus_window(hwnd):
    ref = W.ax_of(hwnd)
    if ref is None:
        return False
    W.activate(W.pid_of(hwnd))
    return W.ax_do(ref, "AXRaise")


def find_all(el, scope, ctype):
    if el is None:
        return []
    pool = children(el) if scope == 2 else walk(el, 1500)
    if ctype is None:
        return pool
    return [e for e in pool if CTYPE.get(role_of(e)) == ctype]


def props(el):
    return CTYPE.get(role_of(el), 50025), " ".join(name_of(el).split()), auto_id(el)


def parent_of(el):
    ref = W.ax_get(el.ref, "AXParent")
    return fetch(ref, el.pid) if ref is not None else None

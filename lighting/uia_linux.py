import threading
import time

from lighting import defaults as D

_state = {"frames": {}, "next": 1, "ids": {}}
_lock = threading.RLock()
CTYPE = {"button": 50000, "checkbox": 50002, "combobox": 50003, "edit": 50004, "link": 50005, "image": 50006,
         "listitem": 50007, "list": 50008, "menuitem": 50011, "radio": 50013, "slider": 50015, "spinner": 50016,
         "tab": 50019, "text": 50020, "treeitem": 50024, "dataitem": 50029, "document": 50030, "splitbutton": 50031,
         "header": 50035, "control": 50025}
FRAME_ROLES = ("FRAME", "DIALOG", "WINDOW", "FILE_CHOOSER", "ALERT")


def A():
    if "A" not in _state:
        import gi
        gi.require_version("Atspi", "2.0")
        from gi.repository import Atspi
        Atspi.init()
        _state["A"] = Atspi
        R = Atspi.Role
        _state["roles"] = {
            R.PUSH_BUTTON: "button", R.TOGGLE_BUTTON: "checkbox", R.CHECK_BOX: "checkbox", R.CHECK_MENU_ITEM: "menuitem",
            R.COMBO_BOX: "combobox", R.ENTRY: "edit", R.PASSWORD_TEXT: "edit", R.TEXT: "edit", R.SPIN_BUTTON: "spinner",
            R.LINK: "link", R.LIST_ITEM: "listitem", R.MENU_ITEM: "menuitem", R.MENU: "menuitem",
            R.RADIO_BUTTON: "radio", R.RADIO_MENU_ITEM: "menuitem", R.SLIDER: "slider", R.PAGE_TAB: "tab",
            R.TREE_ITEM: "treeitem", R.TABLE_CELL: "dataitem", R.TABLE_ROW: "listitem", R.DOCUMENT_WEB: "document",
            R.DOCUMENT_FRAME: "document", R.HEADING: "header", R.LABEL: "text", R.STATIC: "text",
            R.PARAGRAPH: "text", R.LIST: "list", R.LIST_BOX: "list", R.TABLE: "list", R.TREE_TABLE: "list",
            R.TREE: "list", R.IMAGE: "image", R.ICON: "image", R.SWITCH: "checkbox", R.TOOL_BAR: "control",
        }
    return _state["A"]


def api():
    return None, None


def st(acc, name):
    Atspi = A()
    try:
        return acc.get_state_set().contains(getattr(Atspi.StateType, name))
    except Exception:
        return False


def role_of(el):
    Atspi = A()
    try:
        r = el.get_role()
    except Exception:
        return "control"
    role = _state["roles"].get(r, "control")
    if role == "edit" and not st(el, "EDITABLE") and r == Atspi.Role.TEXT:
        return "text"
    return role


def rect_of(el):
    Atspi = A()
    try:
        e = el.get_extents(Atspi.CoordType.SCREEN)
    except Exception:
        return 0, 0, 0, 0
    return e.x, e.y, e.x + e.width, e.y + e.height


live_rect = rect_of


def name_of(el):
    try:
        n = el.get_name() or ""
        if not n.strip() and role_of(el) == "text":
            n = text_value(el)
        return n or el.get_description() or ""
    except Exception:
        return ""


def text_value(el):
    try:
        t = el.get_text_iface()
        if t is not None:
            return t.get_text(0, min(t.get_character_count(), 2000)) or ""
    except Exception:
        pass
    return ""


def kids(el):
    try:
        n = el.get_child_count()
    except Exception:
        return []
    out = []
    for i in range(min(n, 2000)):
        try:
            c = el.get_child_at_index(i)
        except Exception:
            c = None
        if c is not None:
            out.append(c)
    return out


def walk(root, budget_ms=None, limit=4000):
    budget_ms = budget_ms or D.UIA_BUDGET_MS
    t0 = time.perf_counter()
    queue, out = [root], []
    while queue and len(out) < limit:
        el = queue.pop(0)
        out.append(el)
        if not st(el, "SHOWING") and el is not root and role_of(el) in ("list", "control"):
            continue
        queue[0:0] = kids(el)
        if (time.perf_counter() - t0) * 1000 > budget_ms:
            break
    return out[1:]


def apps():
    Atspi = A()
    desk = Atspi.get_desktop(0)
    return kids(desk)


def frame_id(acc):
    try:
        key = (acc.get_process_id(), acc.get_name() or "", acc.get_index_in_parent(), str(acc.get_role()))
    except Exception:
        return 0
    with _lock:
        fid = _state["ids"].get(key)
        if not fid:
            fid = _state["next"]
            _state["next"] += 1
            _state["ids"][key] = fid
        _state["frames"][fid] = acc
    return fid


def frame_list():
    Atspi = A()
    roles = {getattr(Atspi.Role, r) for r in FRAME_ROLES}
    out = []
    for app in apps():
        for f in kids(app):
            try:
                if f.get_role() in roles:
                    out.append(f)
            except Exception:
                continue
    return out


def frames():
    from lighting import win_linux as W
    out = []
    for f in frame_list():
        fid = frame_id(f)
        l, t, r, b = rect_of(f)
        try:
            pid = f.get_process_id()
        except Exception:
            pid = 0
        out.append({"hwnd": fid, "title": f.get_name() or "", "cls": W.class_of_pid(pid), "pid": pid,
                    "exe": W.proc_name(pid), "min": st(f, "ICONIFIED"), "fg": st(f, "ACTIVE")})
    return out


def frame(fid):
    with _lock:
        f = _state["frames"].get(fid)
    if f is None:
        frames()
        f = _state["frames"].get(fid)
    return f


def frame_title(fid):
    f = frame(fid)
    return (f.get_name() or "") if f is not None else ""


def frame_pid(fid):
    f = frame(fid)
    try:
        return f.get_process_id() if f is not None else 0
    except Exception:
        return 0


def frame_rect(fid):
    f = frame(fid)
    return rect_of(f) if f is not None else (0, 0, 0, 0)


def frame_alive(fid):
    f = frame(fid)
    if f is None:
        return False
    try:
        return not st(f, "DEFUNCT") and f.get_role() is not None
    except Exception:
        return False


def frame_close(fid):
    press_named(fid, ("close", "schließen"))


def active_frame():
    for f in frame_list():
        if st(f, "ACTIVE"):
            return frame_id(f)
    return 0


def frame_focus(fid):
    f = frame(fid)
    if f is None:
        return False
    try:
        comp = f.get_component_iface()
        return bool(comp and comp.grab_focus())
    except Exception:
        return False


def match_frame(hwnd):
    from lighting import win_linux as W
    if W.WAYLAND or not W.display():
        return frame(hwnd)
    pid, title = W.x_pid(hwnd), W.x_title(hwnd)
    rx = W.x_rect(hwnd)
    best, score = None, -1
    for f in frame_list():
        try:
            if f.get_process_id() != pid:
                continue
            name = f.get_name() or ""
        except Exception:
            continue
        l, t, r, b = rect_of(f)
        s = (3 if name == title else 1 if name and name in title else 0) + (1 if abs(l - rx[0]) < 60 and abs(t - rx[1]) < 80 else 0)
        if s > score:
            best, score = f, s
    return best


def root_of(hwnd):
    return match_frame(hwnd)


def collect(hwnd, with_text=False, skip_rect=None):
    root = root_of(hwnd)
    if root is None:
        return None, []
    from lighting import win
    wl, wt, wr, wb = win.rect(hwnd)
    items = []
    for el in walk(root):
        role = role_of(el)
        if role in ("control", "list", "image") or (role == "text" and not with_text):
            continue
        if not st(el, "SHOWING"):
            continue
        l, t, r, b = rect_of(el)
        if r - l < 2 or b - t < 2 or r < wl or l > wr or b < wt or t > wb:
            continue
        if skip_rect and l >= skip_rect[0] and t >= skip_rect[1] and r <= skip_rect[2] and b <= skip_rect[3]:
            continue
        items.append(el)
    return root, items


def value_text(el):
    try:
        v = el.get_value_iface()
        if v is not None:
            return "%g" % v.get_current_value()
    except Exception:
        pass
    return text_value(el)


def describe(el):
    role = role_of(el)
    name = " ".join(name_of(el).split())
    s = ""
    if role == "checkbox":
        s += " [-]" if st(el, "INDETERMINATE") else " [x]" if st(el, "CHECKED") or st(el, "PRESSED") else " [ ]"
    if role in ("edit", "combobox", "spinner", "slider"):
        try:
            secret = el.get_role() == A().Role.PASSWORD_TEXT
        except Exception:
            secret = False
        if secret:
            s += " =***"
        else:
            v = " ".join(value_text(el).split())
            if v and v != name:
                s += ' ="%s"' % (v[:40] + "..." if len(v) > 43 else v).replace('"', "'")
    if st(el, "SELECTED") and role in ("listitem", "dataitem", "tab", "treeitem", "radio"):
        s += " *"
    if st(el, "CHECKED") and role == "radio":
        s += " *"
    if st(el, "EXPANDABLE"):
        s += " (expanded)" if st(el, "EXPANDED") else " (collapsed)"
    if not st(el, "ENABLED") and not st(el, "SENSITIVE"):
        s += " (disabled)"
    return role, name, s


def cached(el, key):
    return None


def runtime_id(el):
    try:
        return (el.get_process_id(), el.get_unique_id() if hasattr(el, "get_unique_id") else id(el), rect_of(el))
    except Exception:
        return None


def action_names(el):
    try:
        a = el.get_action_iface()
        if a is None:
            return a, []
        return a, [(a.get_action_name(i) or "").lower() for i in range(a.get_n_actions())]
    except Exception:
        return None, []


def act(el, how=None):
    role = role_of(el)
    if role == "edit":
        return "focused" if focus(el) else None
    a, names = action_names(el)
    for want, label in (("click", "clicked"), ("press", "pressed"), ("activate", "activated"), ("toggle", "toggled"),
                        ("select", "selected"), ("jump", "followed"), ("expand or contract", "toggled open")):
        for i, n in enumerate(names):
            if n == want:
                try:
                    if a.do_action(i):
                        return label
                except Exception:
                    pass
    if names:
        try:
            if a.do_action(0):
                return names[0] or "default action"
        except Exception:
            pass
    try:
        sel = el.get_parent().get_selection_iface()
        if sel is not None and sel.select_child(el.get_index_in_parent()):
            return "selected"
    except Exception:
        pass
    return None


def set_value(el, text):
    try:
        e = el.get_editable_text_iface()
        if e is None or not st(el, "EDITABLE"):
            return False
        return bool(e.set_text_contents(text))
    except Exception:
        return False


def get_value(el):
    v = text_value(el)
    return v if v else None


def main_scroller(hwnd):
    root = root_of(hwnd)
    if root is None:
        return None
    Atspi = A()
    best, area = None, 0
    for el in walk(root, 400):
        try:
            if el.get_role() != Atspi.Role.SCROLL_PANE:
                continue
        except Exception:
            continue
        l, t, r, b = rect_of(el)
        if (r - l) * (b - t) > area:
            best, area = el, (r - l) * (b - t)
    return best


def scroll(el, down=True, large=True):
    Atspi = A()
    for k in kids(el):
        try:
            if k.get_role() != Atspi.Role.SCROLL_BAR or not st(k, "VERTICAL"):
                continue
            v = k.get_value_iface()
            cur, lo, hi = v.get_current_value(), v.get_minimum_value(), v.get_maximum_value()
            step = (hi - lo) * (0.25 if large else 0.08)
            return bool(v.set_current_value(max(lo, min(hi, cur + (step if down else -step)))))
        except Exception:
            continue
    try:
        comp = el.get_component_iface()
        return bool(comp.scroll_to(Atspi.ScrollType.BOTTOM_EDGE if down else Atspi.ScrollType.TOP_EDGE))
    except Exception:
        return False


def element_at(x, y):
    Atspi = A()
    for f in frame_list():
        l, t, r, b = rect_of(f)
        if not (l <= x < r and t <= y < b) or not st(f, "SHOWING"):
            continue
        try:
            el = f.get_component_iface().get_accessible_at_point(int(x), int(y), Atspi.CoordType.SCREEN)
        except Exception:
            el = None
        while el is not None:
            try:
                deeper = el.get_component_iface().get_accessible_at_point(int(x), int(y), Atspi.CoordType.SCREEN)
            except Exception:
                deeper = None
            if deeper is None or deeper == el:
                return el
            el = deeper
    return None


def parent_of(el):
    try:
        return el.get_parent()
    except Exception:
        return None


def page_rect(hwnd):
    root = root_of(hwnd)
    if root is None:
        return None
    for el in walk(root, 300, 800):
        if role_of(el) == "document":
            return rect_of(el)
    return None


def shortcut(el):
    a, names = action_names(el)
    try:
        for i in range(len(names)):
            k = a.get_key_binding(i)
            if k:
                return k.split(";")[-1].replace("<Control>", "ctrl+").replace("<Alt>", "alt+").replace("<Shift>", "shift+").replace("<Primary>", "ctrl+").lower() or None
    except Exception:
        pass
    return None


def texts(el):
    return [t for t in (name_of(el), text_value(el)) if t]


def auto_id(el):
    try:
        attrs = el.get_attributes() or {}
        return attrs.get("id") or el.get_accessible_id() or ""
    except Exception:
        return ""


def toggleable(el):
    return role_of(el) == "checkbox"


def toggled(el):
    return st(el, "CHECKED") or st(el, "PRESSED")


def focused():
    for f in frame_list():
        if not st(f, "ACTIVE"):
            continue
        for el in walk(f, 600):
            if st(el, "FOCUSED"):
                return el
    return None


def pid_of(el):
    try:
        return el.get_process_id()
    except Exception:
        return 0


def focus(el):
    try:
        comp = el.get_component_iface()
        return bool(comp and comp.grab_focus())
    except Exception:
        return False


def focus_window(hwnd):
    f = root_of(hwnd)
    return focus(f) if f is not None else False


def find_all(el, scope, ctype):
    if el is None:
        return []
    pool = kids(el) if scope == 2 else walk(el, 1500)
    if ctype is None:
        return pool
    return [e for e in pool if CTYPE.get(role_of(e)) == ctype]


def props(el):
    return CTYPE.get(role_of(el), 50025), " ".join(name_of(el).split()), auto_id(el)


def press_named(hwnd, names):
    root = root_of(hwnd) if isinstance(hwnd, int) else hwnd
    if root is None:
        return False
    for el in walk(root, 800):
        if role_of(el) == "button" and any(n in name_of(el).lower() for n in names):
            return act(el) is not None
    return False

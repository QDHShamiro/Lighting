import time

from lighting import defaults as D
from lighting.common import Fail

_state = {}

ROLES = {
    50000: "button", 50002: "checkbox", 50003: "combobox", 50004: "edit", 50005: "link", 50007: "listitem",
    50011: "menuitem", 50013: "radio", 50015: "slider", 50016: "spinner", 50019: "tab", 50024: "treeitem",
    50029: "dataitem", 50030: "document", 50031: "splitbutton", 50035: "header",
}
TEXT = 50020


def api():
    if "uia" in _state:
        return _state["uia"], _state["U"]
    import comtypes
    import comtypes.client
    try:
        comtypes.CoInitializeEx(comtypes.COINIT_APARTMENTTHREADED)
    except OSError:
        pass
    comtypes.client.GetModule("UIAutomationCore.dll")
    from comtypes.gen import UIAutomationClient as U
    uia = comtypes.client.CreateObject(U.CUIAutomation8, interface=U.IUIAutomation)
    _state["uia"], _state["U"] = uia, U
    cr = uia.CreateCacheRequest()
    props = [U.UIA_RuntimeIdPropertyId, U.UIA_NamePropertyId, U.UIA_ControlTypePropertyId,
             U.UIA_BoundingRectanglePropertyId, U.UIA_IsEnabledPropertyId, U.UIA_AutomationIdPropertyId,
             U.UIA_ValueValuePropertyId, U.UIA_ToggleToggleStatePropertyId, U.UIA_SelectionItemIsSelectedPropertyId,
             U.UIA_ExpandCollapseExpandCollapseStatePropertyId, U.UIA_IsPasswordPropertyId,
             U.UIA_HasKeyboardFocusPropertyId, U.UIA_ClassNamePropertyId, U.UIA_NativeWindowHandlePropertyId,
             U.UIA_IsValuePatternAvailablePropertyId, U.UIA_IsTogglePatternAvailablePropertyId,
             U.UIA_IsSelectionItemPatternAvailablePropertyId, U.UIA_IsExpandCollapsePatternAvailablePropertyId,
             U.UIA_AcceleratorKeyPropertyId, U.UIA_AccessKeyPropertyId]
    for p in props:
        cr.AddProperty(p)
    _state["cache"] = cr
    return uia, U


def condition(with_text):
    uia, U = api()
    key = "cond-text" if with_text else "cond"
    if key in _state:
        return _state[key]
    types = list(ROLES) + ([TEXT] if with_text else [])
    ors = [uia.CreatePropertyCondition(U.UIA_ControlTypePropertyId, t) for t in types]
    arr = ors[0]
    for c in ors[1:]:
        arr = uia.CreateOrCondition(arr, c)
    cond = uia.CreateAndCondition(uia.CreateAndCondition(uia.ControlViewCondition,
                                                         uia.CreatePropertyCondition(U.UIA_IsOffscreenPropertyId, False)), arr)
    _state[key] = cond
    return cond


def cached(el, pid):
    try:
        v = el.GetCachedPropertyValue(pid)
    except Exception:
        return None
    if isinstance(v, (str, int, float, bool, tuple)):
        return v
    return None


def rect_of(el):
    r = el.CachedBoundingRectangle
    return r.left, r.top, r.right, r.bottom


def runtime_id(el):
    v = cached(el, _state["U"].UIA_RuntimeIdPropertyId)
    return tuple(v) if isinstance(v, tuple) else None


def document_rect(root):
    uia, U = api()
    doc = root.FindFirst(4, uia.CreatePropertyCondition(U.UIA_ControlTypePropertyId, 50030))
    if not doc:
        return None
    r = doc.CurrentBoundingRectangle
    return r.left, r.top, r.right, r.bottom


def collect(hwnd, with_text=False, skip_rect=None):
    from _ctypes import COMError
    uia, U = api()
    try:
        root = uia.ElementFromHandle(hwnd)
        t0 = time.perf_counter()
        arr = root.FindAllBuildCache(4, condition(with_text), _state["cache"])
        items = []
        for i in range(arr.Length):
            el = arr.GetElement(i)
            l, t, r, b = rect_of(el)
            if r - l < 2 or b - t < 2:
                continue
            if skip_rect and l >= skip_rect[0] and t >= skip_rect[1] and r <= skip_rect[2] and b <= skip_rect[3]:
                continue
            items.append(el)
            if (time.perf_counter() - t0) * 1000 > D.UIA_BUDGET_MS:
                break
    except COMError as e:
        code = e.args[0] & 0xFFFFFFFF if e.args and isinstance(e.args[0], int) else 0
        raise Fail("the window's controls could not be read (UI Automation error %#x, the app may be busy)" % code,
                   "lighting snap again in a moment, or lighting read for screen text") from None
    return root, items


def describe(el):
    U = _state["U"]
    ctype = cached(el, U.UIA_ControlTypePropertyId) or 0
    role = ROLES.get(ctype, "text" if ctype == TEXT else "control")
    name = " ".join((cached(el, U.UIA_NamePropertyId) or "").split())
    s = ""
    if cached(el, U.UIA_IsTogglePatternAvailablePropertyId):
        st = cached(el, U.UIA_ToggleToggleStatePropertyId)
        s += " [x]" if st == 1 else " [-]" if st == 2 else " [ ]"
    if cached(el, U.UIA_IsValuePatternAvailablePropertyId) and role in ("edit", "combobox", "document", "spinner", "slider"):
        if cached(el, U.UIA_IsPasswordPropertyId):
            s += " =***"
        else:
            v = cached(el, U.UIA_ValueValuePropertyId)
            if isinstance(v, str) and v.strip():
                v = " ".join(v.split())
                s += ' ="%s"' % (v[:40] + "..." if len(v) > 43 else v).replace('"', "'")
    if cached(el, U.UIA_IsSelectionItemPatternAvailablePropertyId) and cached(el, U.UIA_SelectionItemIsSelectedPropertyId) is True:
        s += " *"
    if cached(el, U.UIA_IsExpandCollapsePatternAvailablePropertyId):
        ec = cached(el, U.UIA_ExpandCollapseExpandCollapseStatePropertyId)
        if ec == 0:
            s += " (collapsed)"
        elif ec == 1:
            s += " (expanded)"
    if cached(el, U.UIA_IsEnabledPropertyId) is False:
        s += " (disabled)"
    return role, name, s


def pattern(el, pid, iface):
    try:
        p = el.GetCurrentPattern(pid)
        return p.QueryInterface(iface) if p else None
    except Exception:
        return None


def act(el, how=None):
    uia, U = api()
    ctype = el.CachedControlType if hasattr(el, "CachedControlType") else el.CurrentControlType
    native = cached(el, U.UIA_NativeWindowHandlePropertyId) or 0
    cls = cached(el, U.UIA_ClassNamePropertyId) or ""
    if native and cls == "Button" and ctype == 50000 and how is None:
        from lighting import win
        win.post_click(native)
        return "posted click"
    order = []
    if ctype == 50002:
        order = ["toggle", "invoke"]
    elif ctype in (50013, 50019, 50007, 50029):
        order = ["select", "invoke"]
    elif ctype in (50011, 50031, 50003, 50024):
        order = ["invoke", "expand", "select"]
    elif ctype in (50004, 50030):
        order = ["focus"]
    else:
        order = ["invoke", "toggle", "select", "expand"]
    order.append("legacy")
    for step in order:
        try:
            if step == "invoke":
                p = pattern(el, U.UIA_InvokePatternId, U.IUIAutomationInvokePattern)
                if p:
                    p.Invoke()
                    return "invoked"
            elif step == "toggle":
                p = pattern(el, U.UIA_TogglePatternId, U.IUIAutomationTogglePattern)
                if p:
                    p.Toggle()
                    return "toggled"
            elif step == "select":
                p = pattern(el, U.UIA_SelectionItemPatternId, U.IUIAutomationSelectionItemPattern)
                if p:
                    p.Select()
                    return "selected"
            elif step == "expand":
                p = pattern(el, U.UIA_ExpandCollapsePatternId, U.IUIAutomationExpandCollapsePattern)
                if p:
                    if p.CurrentExpandCollapseState == 1:
                        p.Collapse()
                        return "collapsed"
                    p.Expand()
                    return "expanded"
            elif step == "focus":
                from lighting import pointer
                if not pointer.front_ok():
                    return None
                el.SetFocus()
                return "focused"
            elif step == "legacy":
                p = pattern(el, U.UIA_LegacyIAccessiblePatternId, U.IUIAutomationLegacyIAccessiblePattern)
                if p:
                    p.DoDefaultAction()
                    return "default action"
        except Exception:
            continue
    return None


def set_value(el, text):
    uia, U = api()
    p = pattern(el, U.UIA_ValuePatternId, U.IUIAutomationValuePattern)
    if not p:
        return False
    try:
        if p.CurrentIsReadOnly:
            return False
        p.SetValue(text)
        return True
    except Exception:
        return False


def get_value(el):
    uia, U = api()
    p = pattern(el, U.UIA_ValuePatternId, U.IUIAutomationValuePattern)
    if p:
        try:
            return p.CurrentValue
        except Exception:
            pass
    tp = pattern(el, U.UIA_TextPatternId, U.IUIAutomationTextPattern)
    if tp:
        try:
            return tp.DocumentRange.GetText(-1)
        except Exception:
            pass
    return None


def main_scroller(hwnd):
    uia, U = api()
    cr = _state.get("scroll-cache")
    if cr is None:
        cr = uia.CreateCacheRequest()
        cr.AddProperty(U.UIA_BoundingRectanglePropertyId)
        _state["scroll-cache"] = cr
    cond = uia.CreateAndCondition(uia.CreatePropertyCondition(U.UIA_IsScrollPatternAvailablePropertyId, True),
                                  uia.CreatePropertyCondition(U.UIA_ScrollVerticallyScrollablePropertyId, True))
    try:
        arr = uia.ElementFromHandle(hwnd).FindAllBuildCache(7, cond, cr)
    except Exception:
        return None
    best, area = None, 0
    for i in range(arr.Length):
        el = arr.GetElement(i)
        l, t, r, b = rect_of(el)
        if (r - l) * (b - t) > area:
            best, area = el, (r - l) * (b - t)
    return best


def scroll(el, down=True, large=True):
    uia, U = api()
    p = pattern(el, U.UIA_ScrollPatternId, U.IUIAutomationScrollPattern)
    if not p:
        return False
    try:
        amount = (3 if large else 4) if down else (0 if large else 1)
        p.Scroll(2, amount)
        return True
    except Exception:
        return False


def element_at(x, y):
    uia, U = api()
    from ctypes import wintypes
    pt = wintypes.POINT(int(x), int(y))
    try:
        return uia.ElementFromPoint(pt)
    except Exception:
        return None


def page_rect(hwnd):
    uia, U = api()
    return document_rect(uia.ElementFromHandle(hwnd))


def shortcut(el):
    U = api()[1]
    return cached(el, U.UIA_AcceleratorKeyPropertyId) or cached(el, U.UIA_AccessKeyPropertyId)


def texts(el):
    U = api()[1]
    return [v for v in (cached(el, U.UIA_NamePropertyId), cached(el, U.UIA_ValueValuePropertyId)) if isinstance(v, str)]


def auto_id(el):
    return cached(el, api()[1].UIA_AutomationIdPropertyId) or ""


def toggleable(el):
    return bool(cached(el, api()[1].UIA_IsTogglePatternAvailablePropertyId))


def toggled(el):
    uia, U = api()
    p = pattern(el, U.UIA_TogglePatternId, U.IUIAutomationTogglePattern)
    return None if p is None else p.CurrentToggleState == 1


def live_rect(el):
    try:
        r = el.CurrentBoundingRectangle
        return r.left, r.top, r.right, r.bottom
    except Exception:
        return rect_of(el)


def focused():
    try:
        return api()[0].GetFocusedElement()
    except Exception:
        return None


def pid_of(el):
    try:
        return el.CurrentProcessId
    except Exception:
        return 0


def focus(el):
    try:
        el.SetFocus()
        return True
    except Exception:
        return False


def native_of(el):
    uia, U = api()
    try:
        return cached(el, U.UIA_NativeWindowHandlePropertyId) or el.CurrentNativeWindowHandle or 0
    except Exception:
        return 0


def from_handle(hwnd):
    try:
        return api()[0].ElementFromHandle(hwnd)
    except Exception:
        return None


def focus_window(hwnd):
    try:
        api()[0].ElementFromHandle(hwnd).SetFocus()
        return True
    except Exception:
        return False


_conds = {}


def find_all(el, scope, ctype):
    uia, U = api()
    if ctype not in _conds:
        _conds[ctype] = uia.CreateTrueCondition() if ctype is None else uia.CreatePropertyCondition(U.UIA_ControlTypePropertyId, ctype)
    arr = el.FindAllBuildCache(scope, _conds[ctype], _state["cache"])
    return [arr.GetElement(i) for i in range(arr.Length)]


def props(el):
    U = api()[1]
    return (cached(el, U.UIA_ControlTypePropertyId), " ".join((cached(el, U.UIA_NamePropertyId) or "").split()),
            cached(el, U.UIA_AutomationIdPropertyId) or "")


def root_of(hwnd):
    return api()[0].ElementFromHandle(hwnd)

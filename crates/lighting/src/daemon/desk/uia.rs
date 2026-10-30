use std::cell::RefCell;
use std::mem::ManuallyDrop;
use std::time::Instant;

use windows::Win32::Foundation::VARIANT_BOOL;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::System::Variant::{
    VARENUM, VARIANT, VARIANT_0, VARIANT_0_0, VARIANT_0_0_0, VT_BOOL, VT_BSTR, VT_I4, VariantClear, VariantToInt32Array,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation8, ExpandCollapseState_Expanded, IUIAutomation, IUIAutomation2, IUIAutomationCacheRequest,
    IUIAutomationCondition, IUIAutomationElement, IUIAutomationExpandCollapsePattern, IUIAutomationInvokePattern,
    IUIAutomationLegacyIAccessiblePattern, IUIAutomationScrollPattern, IUIAutomationSelectionItemPattern,
    IUIAutomationTextPattern, IUIAutomationTogglePattern, IUIAutomationValuePattern, ScrollAmount,
    ScrollAmount_LargeDecrement, ScrollAmount_LargeIncrement, ScrollAmount_NoAmount, TreeScope, TreeScope_Children,
    TreeScope_Descendants, TreeScope_Subtree, UIA_AcceleratorKeyPropertyId, UIA_AccessKeyPropertyId,
    UIA_AutomationIdPropertyId, UIA_BoundingRectanglePropertyId, UIA_ClassNamePropertyId, UIA_ControlTypePropertyId,
    UIA_ExpandCollapseExpandCollapseStatePropertyId, UIA_ExpandCollapsePatternId, UIA_HasKeyboardFocusPropertyId,
    UIA_InvokePatternId, UIA_IsEnabledPropertyId, UIA_IsExpandCollapsePatternAvailablePropertyId,
    UIA_IsOffscreenPropertyId, UIA_IsPasswordPropertyId, UIA_IsScrollPatternAvailablePropertyId,
    UIA_IsSelectionItemPatternAvailablePropertyId, UIA_IsTogglePatternAvailablePropertyId,
    UIA_IsValuePatternAvailablePropertyId, UIA_LegacyIAccessiblePatternId, UIA_NamePropertyId,
    UIA_NativeWindowHandlePropertyId, UIA_PATTERN_ID, UIA_PROPERTY_ID, UIA_RuntimeIdPropertyId, UIA_ScrollPatternId,
    UIA_ScrollVerticallyScrollablePropertyId, UIA_SelectionItemIsSelectedPropertyId, UIA_SelectionItemPatternId,
    UIA_TextPatternId, UIA_TogglePatternId, UIA_ToggleToggleStatePropertyId, UIA_ValuePatternId,
    UIA_ValueValuePropertyId,
};
use windows::core::{BSTR, Interface};

use super::win::{Rect, h};

const BUDGET_MS: u128 = 1500;
const CONNECT_MS: u32 = 2000;
const TRANSACTION_MS: u32 = 3000;
const DOCUMENT: i32 = 50030;
const TEXT: i32 = 50020;
pub const ROLES: &[(i32, &str)] = &[
    (50000, "button"),
    (50002, "checkbox"),
    (50003, "combobox"),
    (50004, "edit"),
    (50005, "link"),
    (50007, "listitem"),
    (50011, "menuitem"),
    (50013, "radio"),
    (50015, "slider"),
    (50016, "spinner"),
    (50019, "tab"),
    (50024, "treeitem"),
    (50029, "dataitem"),
    (50030, "document"),
    (50031, "splitbutton"),
    (50035, "header"),
];
const CACHED: &[UIA_PROPERTY_ID] = &[
    UIA_RuntimeIdPropertyId,
    UIA_NamePropertyId,
    UIA_ControlTypePropertyId,
    UIA_BoundingRectanglePropertyId,
    UIA_IsEnabledPropertyId,
    UIA_AutomationIdPropertyId,
    UIA_ValueValuePropertyId,
    UIA_ToggleToggleStatePropertyId,
    UIA_SelectionItemIsSelectedPropertyId,
    UIA_ExpandCollapseExpandCollapseStatePropertyId,
    UIA_IsPasswordPropertyId,
    UIA_HasKeyboardFocusPropertyId,
    UIA_ClassNamePropertyId,
    UIA_NativeWindowHandlePropertyId,
    UIA_IsValuePatternAvailablePropertyId,
    UIA_IsTogglePatternAvailablePropertyId,
    UIA_IsSelectionItemPatternAvailablePropertyId,
    UIA_IsExpandCollapsePatternAvailablePropertyId,
    UIA_AcceleratorKeyPropertyId,
    UIA_AccessKeyPropertyId,
];

struct Api {
    uia: IUIAutomation,
    cache: IUIAutomationCacheRequest,
    plain: IUIAutomationCondition,
    texty: IUIAutomationCondition,
    scrolls: IUIAutomationCondition,
    rects: IUIAutomationCacheRequest,
}

thread_local! {
    static API: RefCell<Option<Api>> = const { RefCell::new(None) };
}

#[derive(Clone)]
pub struct El(IUIAutomationElement);

struct Var(VARIANT);

impl Drop for Var {
    fn drop(&mut self) {
        let _ = unsafe { VariantClear(&raw mut self.0) };
    }
}

impl Var {
    fn kind(&self) -> VARENUM {
        unsafe { self.0.Anonymous.Anonymous.vt }
    }

    fn int(&self) -> Option<i32> {
        (self.kind() == VT_I4).then(|| unsafe { self.0.Anonymous.Anonymous.Anonymous.lVal })
    }

    fn flag(&self) -> Option<bool> {
        (self.kind() == VT_BOOL).then(|| unsafe { self.0.Anonymous.Anonymous.Anonymous.boolVal.0 != 0 })
    }

    fn text(&self) -> Option<String> {
        (self.kind() == VT_BSTR).then(|| unsafe { self.0.Anonymous.Anonymous.Anonymous.bstrVal.to_string() })
    }

    fn ints(&self) -> Option<Vec<i32>> {
        let mut buf = [0i32; 16];
        let mut n = 0u32;
        unsafe { VariantToInt32Array(&raw const self.0, &mut buf, &raw mut n) }.ok()?;
        Some(buf[..n as usize].to_vec())
    }
}

fn variant(vt: VARENUM, value: VARIANT_0_0_0) -> VARIANT {
    let inner = VARIANT_0_0 { vt, wReserved1: 0, wReserved2: 0, wReserved3: 0, Anonymous: value };
    VARIANT { Anonymous: VARIANT_0 { Anonymous: ManuallyDrop::new(inner) } }
}

fn int_var(v: i32) -> VARIANT {
    variant(VT_I4, VARIANT_0_0_0 { lVal: v })
}

fn bool_var(v: bool) -> VARIANT {
    variant(VT_BOOL, VARIANT_0_0_0 { boolVal: VARIANT_BOOL(if v { -1 } else { 0 }) })
}

fn open() -> windows::core::Result<Api> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let uia: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
        if let Ok(two) = uia.cast::<IUIAutomation2>() {
            let _ = two.SetAutoSetFocus(false);
            let _ = two.SetConnectionTimeout(CONNECT_MS);
            let _ = two.SetTransactionTimeout(TRANSACTION_MS);
        }
        let cache = uia.CreateCacheRequest()?;
        for p in CACHED {
            cache.AddProperty(*p)?;
        }
        let rects = uia.CreateCacheRequest()?;
        rects.AddProperty(UIA_BoundingRectanglePropertyId)?;
        let types = |with_text: bool| -> windows::core::Result<IUIAutomationCondition> {
            let mut ors = Vec::new();
            for t in ROLES.iter().map(|(t, _)| *t).chain(with_text.then_some(TEXT)) {
                ors.push(Some(uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &int_var(t))?));
            }
            let visible = uia.CreateAndCondition(
                &uia.ControlViewCondition()?,
                &uia.CreatePropertyCondition(UIA_IsOffscreenPropertyId, &bool_var(false))?,
            )?;
            uia.CreateAndCondition(&visible, &uia.CreateOrConditionFromNativeArray(&ors)?)
        };
        let (plain, texty) = (types(false)?, types(true)?);
        let scrolls = uia.CreateAndCondition(
            &uia.CreatePropertyCondition(UIA_IsScrollPatternAvailablePropertyId, &bool_var(true))?,
            &uia.CreatePropertyCondition(UIA_ScrollVerticallyScrollablePropertyId, &bool_var(true))?,
        )?;
        Ok(Api { uia, cache, plain, texty, scrolls, rects })
    }
}

fn with<T>(f: impl FnOnce(&Api) -> windows::core::Result<T>) -> Option<T> {
    API.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = open().ok();
        }
        slot.as_ref().and_then(|api| f(api).ok())
    })
}

fn find(
    root: &IUIAutomationElement,
    scope: TreeScope,
    cond: &IUIAutomationCondition,
    cache: &IUIAutomationCacheRequest,
) -> windows::core::Result<Vec<El>> {
    let arr = unsafe { root.FindAllBuildCache(scope, cond, cache)? };
    let n = unsafe { arr.Length()? };
    Ok((0..n).filter_map(|i| unsafe { arr.GetElement(i) }.ok().map(El)).collect())
}

pub fn collect(hwnd: isize, with_text: bool, skip: Option<Rect>) -> Vec<El> {
    let t0 = Instant::now();
    with(|api| {
        let root = unsafe { api.uia.ElementFromHandle(h(hwnd))? };
        let cond = if with_text { &api.texty } else { &api.plain };
        let arr = unsafe { root.FindAllBuildCache(TreeScope_Descendants, cond, &api.cache)? };
        let n = unsafe { arr.Length()? };
        let mut items = Vec::new();
        for i in 0..n {
            let Ok(el) = (unsafe { arr.GetElement(i) }).map(El) else { continue };
            let (l, t, r, b) = el.rect();
            if r - l < 2 || b - t < 2 {
                continue;
            }
            if skip.is_some_and(|(sl, st, sr, sb)| l >= sl && t >= st && r <= sr && b <= sb) {
                continue;
            }
            items.push(el);
            if t0.elapsed().as_millis() > BUDGET_MS {
                break;
            }
        }
        Ok(items)
    })
    .unwrap_or_default()
}

pub fn page_rect(hwnd: isize) -> Option<Rect> {
    with(|api| unsafe {
        let root = api.uia.ElementFromHandle(h(hwnd))?;
        let doc = root.FindFirst(
            TreeScope_Descendants,
            &api.uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &int_var(DOCUMENT))?,
        )?;
        let r = doc.CurrentBoundingRectangle()?;
        Ok((r.left, r.top, r.right, r.bottom))
    })
}

pub fn main_scroller(hwnd: isize) -> Option<El> {
    with(|api| {
        let root = unsafe { api.uia.ElementFromHandle(h(hwnd))? };
        let all = find(&root, TreeScope_Subtree, &api.scrolls, &api.rects)?;
        let area = |el: &El| {
            let (l, t, r, b) = el.rect();
            i64::from(r - l) * i64::from(b - t)
        };
        Ok(all.into_iter().max_by_key(area))
    })
    .flatten()
}

pub fn find_all(el: &El, children: bool, ctype: Option<i32>) -> Vec<El> {
    with(|api| {
        let cond = match ctype {
            Some(t) => unsafe { api.uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &int_var(t))? },
            None => unsafe { api.uia.CreateTrueCondition()? },
        };
        find(&el.0, if children { TreeScope_Children } else { TreeScope_Descendants }, &cond, &api.cache)
    })
    .unwrap_or_default()
}

pub fn focused() -> Option<El> {
    with(|api| unsafe { api.uia.GetFocusedElement() }.map(El))
}

pub fn from_handle(hwnd: isize) -> Option<El> {
    with(|api| unsafe { api.uia.ElementFromHandle(h(hwnd)) }.map(El))
}

pub fn role_name(ctype: i32) -> &'static str {
    ROLES.iter().find(|(t, _)| *t == ctype).map_or(if ctype == TEXT { "text" } else { "control" }, |(_, r)| r)
}

impl El {
    fn cached(&self, id: UIA_PROPERTY_ID) -> Option<Var> {
        unsafe { self.0.GetCachedPropertyValue(id) }.ok().map(Var)
    }

    fn int(&self, id: UIA_PROPERTY_ID) -> Option<i32> {
        self.cached(id).and_then(|v| v.int())
    }

    fn flag(&self, id: UIA_PROPERTY_ID) -> Option<bool> {
        self.cached(id).and_then(|v| v.flag())
    }

    fn text(&self, id: UIA_PROPERTY_ID) -> Option<String> {
        self.cached(id).and_then(|v| v.text())
    }

    pub fn ctype(&self) -> i32 {
        self.int(UIA_ControlTypePropertyId).unwrap_or(0)
    }

    pub fn role(&self) -> &'static str {
        role_name(self.ctype())
    }

    pub fn name(&self) -> String {
        self.text(UIA_NamePropertyId).unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ")
    }

    pub fn aid(&self) -> String {
        self.text(UIA_AutomationIdPropertyId).unwrap_or_default()
    }

    pub fn class(&self) -> String {
        self.text(UIA_ClassNamePropertyId).unwrap_or_default()
    }

    pub fn rect(&self) -> Rect {
        unsafe { self.0.CachedBoundingRectangle() }.map_or((0, 0, 0, 0), |r| (r.left, r.top, r.right, r.bottom))
    }

    pub fn live_rect(&self) -> Rect {
        unsafe { self.0.CurrentBoundingRectangle() }
            .map_or_else(|_| self.rect(), |r| (r.left, r.top, r.right, r.bottom))
    }

    pub fn runtime_id(&self) -> Option<Vec<i32>> {
        self.cached(UIA_RuntimeIdPropertyId).and_then(|v| v.ints())
    }

    pub fn shortcut(&self) -> Option<String> {
        [UIA_AcceleratorKeyPropertyId, UIA_AccessKeyPropertyId]
            .into_iter()
            .find_map(|id| self.text(id).filter(|s| !s.is_empty()))
    }

    pub fn texts(&self) -> Vec<String> {
        [UIA_NamePropertyId, UIA_ValueValuePropertyId].into_iter().filter_map(|id| self.text(id)).collect()
    }

    pub fn toggleable(&self) -> bool {
        self.flag(UIA_IsTogglePatternAvailablePropertyId).unwrap_or(false)
    }

    pub fn describe(&self) -> (&'static str, String, String) {
        let role = self.role();
        let mut s = String::new();
        if self.toggleable() {
            s += match self.int(UIA_ToggleToggleStatePropertyId) {
                Some(1) => " [x]",
                Some(2) => " [-]",
                _ => " [ ]",
            };
        }
        if self.flag(UIA_IsValuePatternAvailablePropertyId) == Some(true)
            && ["edit", "combobox", "document", "spinner", "slider"].contains(&role)
        {
            if self.flag(UIA_IsPasswordPropertyId) == Some(true) {
                s += " =***";
            } else if let Some(v) = self.text(UIA_ValueValuePropertyId).filter(|v| !v.trim().is_empty()) {
                let v = v.split_whitespace().collect::<Vec<_>>().join(" ");
                let shown: String =
                    if v.chars().count() > 43 { v.chars().take(40).collect::<String>() + "..." } else { v };
                s += &format!(" =\"{}\"", shown.replace('"', "'"));
            }
        }
        if self.flag(UIA_IsSelectionItemPatternAvailablePropertyId) == Some(true)
            && self.flag(UIA_SelectionItemIsSelectedPropertyId) == Some(true)
        {
            s += " *";
        }
        if self.flag(UIA_IsExpandCollapsePatternAvailablePropertyId) == Some(true) {
            match self.int(UIA_ExpandCollapseExpandCollapseStatePropertyId) {
                Some(0) => s += " (collapsed)",
                Some(1) => s += " (expanded)",
                _ => {}
            }
        }
        if self.flag(UIA_IsEnabledPropertyId) == Some(false) {
            s += " (disabled)";
        }
        (role, self.name(), s)
    }

    fn pattern<T: Interface>(&self, id: UIA_PATTERN_ID) -> Option<T> {
        unsafe { self.0.GetCurrentPatternAs::<T>(id) }.ok()
    }

    pub fn native(&self) -> isize {
        self.int(UIA_NativeWindowHandlePropertyId)
            .map(|n| n as isize)
            .filter(|n| *n != 0)
            .or_else(|| unsafe { self.0.CurrentNativeWindowHandle() }.ok().map(|w| w.0 as isize))
            .unwrap_or(0)
    }

    pub fn act(&self, may_focus: bool) -> Option<&'static str> {
        let ctype = self.ctype();
        if ctype == 50000 && self.native() != 0 && self.class() == "Button" {
            super::win::post_click(self.native());
            return Some("posted click");
        }
        let order: &[&str] = match ctype {
            50002 => &["toggle", "invoke"],
            50013 | 50019 | 50007 | 50029 => &["select", "invoke"],
            50011 | 50031 | 50003 | 50024 => &["invoke", "expand", "select"],
            50004 | 50030 => &["focus"],
            _ => &["invoke", "toggle", "select", "expand"],
        };
        order.iter().chain(&["legacy"]).find_map(|step| self.step(step, may_focus))
    }

    fn step(&self, step: &str, may_focus: bool) -> Option<&'static str> {
        unsafe {
            match step {
                "invoke" => {
                    self.pattern::<IUIAutomationInvokePattern>(UIA_InvokePatternId)?.Invoke().ok().map(|()| "invoked")
                }
                "toggle" => {
                    self.pattern::<IUIAutomationTogglePattern>(UIA_TogglePatternId)?.Toggle().ok().map(|()| "toggled")
                }
                "select" => self
                    .pattern::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)?
                    .Select()
                    .ok()
                    .map(|()| "selected"),
                "expand" => {
                    let p = self.pattern::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)?;
                    if p.CurrentExpandCollapseState().ok() == Some(ExpandCollapseState_Expanded) {
                        p.Collapse().ok().map(|()| "collapsed")
                    } else {
                        p.Expand().ok().map(|()| "expanded")
                    }
                }
                "focus" if may_focus => self.0.SetFocus().ok().map(|()| "focused"),
                "legacy" => self
                    .pattern::<IUIAutomationLegacyIAccessiblePattern>(UIA_LegacyIAccessiblePatternId)?
                    .DoDefaultAction()
                    .ok()
                    .map(|()| "default action"),
                _ => None,
            }
        }
    }

    pub fn set_value(&self, text: &str) -> bool {
        let Some(p) = self.pattern::<IUIAutomationValuePattern>(UIA_ValuePatternId) else { return false };
        unsafe {
            if p.CurrentIsReadOnly().is_ok_and(|r| r.as_bool()) {
                return false;
            }
            p.SetValue(&BSTR::from(text)).is_ok()
        }
    }

    pub fn value(&self) -> Option<String> {
        if let Some(p) = self.pattern::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            && let Ok(v) = unsafe { p.CurrentValue() }
        {
            return Some(v.to_string());
        }
        let p = self.pattern::<IUIAutomationTextPattern>(UIA_TextPatternId)?;
        unsafe { p.DocumentRange().and_then(|r| r.GetText(-1)) }.ok().map(|t| t.to_string())
    }

    pub fn scroll(&self, down: bool) -> bool {
        let Some(p) = self.pattern::<IUIAutomationScrollPattern>(UIA_ScrollPatternId) else { return false };
        let amount: ScrollAmount = if down { ScrollAmount_LargeIncrement } else { ScrollAmount_LargeDecrement };
        unsafe { p.Scroll(ScrollAmount_NoAmount, amount) }.is_ok()
    }

    pub fn pid(&self) -> u32 {
        unsafe { self.0.CurrentProcessId() }.map_or(0, |p| p as u32)
    }

    pub fn focus(&self) -> bool {
        unsafe { self.0.SetFocus() }.is_ok()
    }
}

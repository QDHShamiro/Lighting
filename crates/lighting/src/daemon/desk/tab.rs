use lt_core::{Call, Fail, Flag};

use super::super::Daemon;
use super::super::ctx::Ctx;
use super::look::grab_area;
use super::{Desk, Halt, act, ocr, state, win};

const WT_CLASS: &str = "CASCADIA_HOSTING_WINDOW_CLASS";

pub fn find_tab(title: &str) -> Option<isize> {
    win::windows().into_iter().find(|w| w.title == title && w.cls == WT_CLASS).map(|w| w.hwnd)
}

pub fn tab_text(hwnd: isize) -> String {
    if win::minimized(hwnd) {
        return String::new();
    }
    let Ok(img) = grab_area(win::rect(hwnd), hwnd) else { return String::new() };
    ocr::recognize(&img, "auto")
        .map(|lines| lines.into_iter().map(|l| l.0).collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

pub fn type_prompt(d: &Daemon, ctx: &mut Ctx, hwnd: isize, prompt: &str) -> Result<String, Fail> {
    let mut call = Call { name: "type".into(), pos: vec!["focused".into(), prompt.into()], ..Call::default() };
    call.flags.insert("enter".into(), Flag::On);
    state::with(ctx, |st, ctx| {
        st.hwnd = hwnd;
        ctx.target = Some(("app".into(), hwnd.to_string()));
        act::type_in(&mut Desk { d, st, ctx }, &call)
    })
    .map_err(|h| match h {
        Halt::Fail(f) => f,
        Halt::Busy => Fail::hint(
            "you kept using mouse or keyboard, so the prompt was not typed",
            "type it in the new tab, or lighting type focused \"...\" --enter",
        ),
    })
}

use super::combo::{self, ALT, CTRL, Combo};
use super::uia::{self, El};
use super::{Desk, focus, input, win};

const TAB: i32 = 50018;
const TAB_ITEM: i32 = 50019;
const STEPS: &[(&str, i32)] = &[("ctrl+tab", 1), ("ctrl+pagedown", 1), ("ctrl+shift+tab", -1), ("ctrl+pageup", -1)];
const NAME_CHARS: usize = 40;

fn mods(c: &Combo) -> Vec<u16> {
    let mut m = c.mods.clone();
    m.sort_unstable();
    m.dedup();
    m
}

fn same(a: &Combo, b: &Combo) -> bool {
    a.vk == b.vk && mods(a) == mods(b)
}

fn step_of(c: &Combo) -> Option<i32> {
    STEPS.iter().find(|(s, _)| combo::parse(s, input::scan).is_ok_and(|s| same(&s, c))).map(|(_, n)| *n)
}

fn digit(c: &Combo) -> Option<usize> {
    c.main.parse().ok().filter(|n| (1..=9).contains(n))
}

fn short(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(NAME_CHARS).collect()
}

pub fn tab_key(c: &Combo) -> bool {
    step_of(c).is_some() || (digit(c).is_some() && (mods(c) == [CTRL] || mods(c) == [CTRL, ALT]))
}

pub fn tab_strip(hwnd: isize) -> Vec<El> {
    let Some(root) = uia::from_handle(hwnd) else { return Vec::new() };
    uia::find_all(&root, false, Some(TAB))
        .iter()
        .map(|t| uia::find_all(t, false, Some(TAB_ITEM)))
        .max_by_key(Vec::len)
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| uia::find_all(&root, false, Some(TAB_ITEM)))
}

fn select_tab(hwnd: isize, c: &Combo) -> Option<String> {
    let index = digit(c).filter(|_| mods(c) == [CTRL, ALT]);
    let step = step_of(c);
    if index.is_none() && step.is_none() {
        return None;
    }
    let items = tab_strip(hwnd);
    let n = items.len();
    if n < 2 {
        return None;
    }
    let at = items.iter().position(El::selected).unwrap_or(0);
    let want = match index {
        Some(9) => n - 1,
        Some(i) => i.min(n) - 1,
        None => (at as i64 + i64::from(step?)).rem_euclid(n as i64) as usize,
    };
    let el = &items[want];
    el.select().then(|| format!("tab {}/{n} \"{}\" selected", want + 1, short(&el.name())))
}

pub fn by_ui(hwnd: isize, c: &Combo) -> Option<(String, Option<&'static str>)> {
    if Desk::is_browser(hwnd) {
        return None;
    }
    let prev = win::foreground();
    let how = select_tab(hwnd, c)?;
    let back = focus::put_back(prev, hwnd);
    Some((how, back))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(text: &str) -> Combo {
        combo::parse(text, input::scan).unwrap()
    }

    #[test]
    fn tab_keys_are_known_in_any_spelling() {
        for yes in ["ctrl+tab", "Ctrl+Shift+Tab", "strg+tab", "ctrl+pagedown", "ctrl+pageup", "ctrl+alt+3", "ctrl+9"] {
            assert!(tab_key(&c(yes)), "{yes}");
        }
        for no in ["ctrl+s", "alt+tab", "ctrl+alt+0", "shift+tab", "tab"] {
            assert!(!tab_key(&c(no)), "{no}");
        }
        assert_eq!(
            (step_of(&c("ctrl+tab")), step_of(&c("ctrl+shift+tab")), step_of(&c("ctrl+3"))),
            (Some(1), Some(-1), None)
        );
    }
}

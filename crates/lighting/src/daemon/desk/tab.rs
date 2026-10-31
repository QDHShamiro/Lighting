use super::look::grab_area;
use super::{ocr, win};

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

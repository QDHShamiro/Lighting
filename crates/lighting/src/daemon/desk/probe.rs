use super::{grab, input, ocr, win};

pub fn titled(title: &str) -> Vec<isize> {
    win::windows().into_iter().filter(|w| w.title == title).map(|w| w.hwnd).collect()
}

pub fn close(hwnd: isize) {
    win::close(hwnd);
}

pub fn ocr_text(hwnd: isize) -> Result<String, String> {
    let img = grab::print_window(hwnd).ok_or("PrintWindow returned nothing")?;
    let lines = ocr::recognize(&img, "en").map_err(|f| f.text())?;
    Ok(lines.iter().map(|l| l.0.as_str()).collect::<Vec<_>>().join(" "))
}

pub fn cursor() -> (i32, i32) {
    input::cursor()
}

pub fn idle_ms() -> u32 {
    input::idle_ms()
}

pub fn foreground() -> isize {
    win::foreground()
}

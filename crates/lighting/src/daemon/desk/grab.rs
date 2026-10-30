use image::{DynamicImage, RgbImage, imageops};
use windows::Win32::Graphics::Gdi::{
    BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GetDC, GetDIBits, HDC, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PW_RENDERFULLCONTENT, PrintWindow};

use super::win::{Rect, h, minimized, outer_rect, rect};

const DARK: u8 = 8;

fn bitmap(w: i32, ht: i32, draw: impl FnOnce(HDC, HDC) -> bool) -> Option<RgbImage> {
    if w <= 0 || ht <= 0 {
        return None;
    }
    let mut pixels = vec![0u8; w as usize * ht as usize * 4];
    let ok = unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bmp = CreateCompatibleBitmap(screen, w, ht);
        let old = SelectObject(dc, bmp.into());
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -ht,
                biPlanes: 1,
                biBitCount: 32,
                ..Default::default()
            },
            ..Default::default()
        };
        let ok = draw(dc, screen)
            && GetDIBits(dc, bmp, 0, ht as u32, Some(pixels.as_mut_ptr().cast()), &raw mut info, DIB_RGB_COLORS) == ht;
        SelectObject(dc, old);
        let _ = DeleteObject(bmp.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        ok
    };
    if !ok {
        return None;
    }
    let rgb: Vec<u8> = pixels.as_chunks::<4>().0.iter().flat_map(|[b, g, r, _]| [*r, *g, *b]).collect();
    RgbImage::from_raw(w as u32, ht as u32, rgb)
}

fn dark(img: &RgbImage) -> bool {
    let small = DynamicImage::ImageRgb8(img.clone()).thumbnail(32, 32).to_luma8();
    small.pixels().map(|p| p.0[0]).max().unwrap_or(0) < DARK
}

pub fn print_window(hwnd: isize) -> Option<RgbImage> {
    if minimized(hwnd) {
        return None;
    }
    let (wl, wt, wr, wb) = outer_rect(hwnd)?;
    let full = bitmap(wr - wl, wb - wt, |dc, _| unsafe {
        PrintWindow(h(hwnd), dc, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)).as_bool()
    })?;
    let (l, t, r, b) = rect(hwnd);
    let x = (l - wl).clamp(0, full.width() as i32) as u32;
    let y = (t - wt).clamp(0, full.height() as i32) as u32;
    let cw = ((r - l).max(1) as u32).min(full.width() - x);
    let ch = ((b - t).max(1) as u32).min(full.height() - y);
    let img = imageops::crop_imm(&full, x, y, cw, ch).to_image();
    (!dark(&img)).then_some(img)
}

pub fn screen(area: Rect) -> Option<RgbImage> {
    let (l, t, r, b) = area;
    bitmap(r - l, b - t, |dc, screen| unsafe {
        BitBlt(dc, 0, 0, r - l, b - t, Some(screen), l, t, SRCCOPY | CAPTUREBLT).is_ok()
    })
}

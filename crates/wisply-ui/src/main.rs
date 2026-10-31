#![cfg_attr(windows, windows_subsystem = "windows")]

mod assets;
mod bridge;

use std::borrow::Cow;

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::{Icon, WindowBuilder};
use wry::WebViewBuilder;
use wry::http::Response;

const TITLE: &str = "Wisply";
const SIZE: (f64, f64) = (1280.0, 820.0);
const MIN_SIZE: (f64, f64) = (720.0, 480.0);
const START: &str = "wisply://localhost/";
const BACKGROUND: (u8, u8, u8, u8) = (5, 5, 5, 255);
const QUIET_ENV: &str = "WISPLY_UI_QUIET";
const HELP: &str = "the Wisply window, open it with wisply ui";
const ICON_SMALL: &[u8] = include_bytes!("../icons/32.png");
const ICON_BIG: &[u8] = include_bytes!("../icons/256.png");

pub enum Ev {
    Reply(u64, i32, String),
}

fn ours(url: &str) -> bool {
    let rest = url.strip_prefix("wisply://localhost").or_else(|| url.strip_prefix("http://wisply.localhost"));
    rest.is_some_and(|r| r.is_empty() || r.starts_with('/')) || url == "about:blank"
}

fn icon(png: &[u8]) -> Option<Icon> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?.into_rgba8();
    let (w, h) = img.dimensions();
    Icon::from_rgba(img.into_raw(), w, h).ok()
}

fn main() -> wry::Result<()> {
    if let Some(arg) = std::env::args().nth(1).filter(|a| ["--version", "-V", "--help", "-h"].contains(&a.as_str())) {
        let version = format!("wisply-ui {}", env!("CARGO_PKG_VERSION"));
        println!("{}", if arg.contains('V') || arg == "--version" { version } else { format!("{version}: {HELP}") });
        return Ok(());
    }
    let home = wisply_proto::home();
    let event_loop = EventLoopBuilder::<Ev>::with_user_event().build();
    let proxy = event_loop.create_proxy();
    let builder = WindowBuilder::new()
        .with_title(TITLE)
        .with_window_icon(icon(if cfg!(windows) { ICON_SMALL } else { ICON_BIG }))
        .with_inner_size(LogicalSize::new(SIZE.0, SIZE.1))
        .with_min_inner_size(LogicalSize::new(MIN_SIZE.0, MIN_SIZE.1))
        .with_focused(std::env::var_os(QUIET_ENV).is_none());
    #[cfg(windows)]
    let builder = {
        use tao::platform::windows::WindowBuilderExtWindows;
        builder.with_taskbar_icon(icon(ICON_BIG))
    };
    let window = builder.build(&event_loop).map_err(|e| wry::Error::Io(std::io::Error::other(e.to_string())))?;
    let builder = WebViewBuilder::new()
        .with_background_color(BACKGROUND)
        .with_custom_protocol("wisply".into(), |_, req| {
            let (status, kind, body) = assets::serve(req.uri().path());
            Response::builder()
                .status(status)
                .header("Content-Type", kind)
                .header("Content-Security-Policy", assets::CSP)
                .header("Cache-Control", "no-store")
                .body(Cow::Borrowed(body))
                .unwrap_or_else(|_| Response::new(Cow::Borrowed(&[][..])))
        })
        .with_ipc_handler(move |req| bridge::ask(&home, &proxy, req.body()))
        .with_navigation_handler(|url| ours(&url))
        .with_new_window_req_handler(|url, _| {
            bridge::open_outside(&url);
            wry::NewWindowResponse::Deny
        })
        .with_focused(std::env::var_os(QUIET_ENV).is_none())
        .with_url(START);
    #[cfg(any(windows, target_os = "macos"))]
    let webview = builder.build(&window)?;
    #[cfg(not(any(windows, target_os = "macos")))]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        let vbox = window.default_vbox().ok_or(wry::Error::MessageSender)?;
        builder.build_gtk(vbox)?
    };
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(Ev::Reply(id, code, out)) => {
                let text = serde_json::to_string(&out).unwrap_or_else(|_| "\"\"".into());
                let _ = webview.evaluate_script(&format!("window.lt && window.lt.done({id}, {code}, {text})"));
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => *flow = ControlFlow::Exit,
            _ => {}
        }
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_icons_decode() {
        assert!(super::icon(super::ICON_SMALL).is_some() && super::icon(super::ICON_BIG).is_some());
    }

    #[test]
    fn only_own_pages_load_inside_the_window() {
        assert!(super::ours("wisply://localhost/") && super::ours("http://wisply.localhost/app.js"));
        assert!(!super::ours("https://example.com") && !super::ours("http://wisply.localhost.evil.com/"));
    }
}

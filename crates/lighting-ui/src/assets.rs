pub const CSP: &str = "default-src 'self' lighting: http://lighting.localhost; script-src 'self' lighting: http://lighting.localhost; \
                       style-src 'self' 'unsafe-inline' lighting: http://lighting.localhost; img-src 'self' data: lighting: http://lighting.localhost; \
                       connect-src 'none'; worker-src blob:; object-src 'none'; base-uri 'none'; form-action 'none'";

const FILES: &[(&str, &str, &[u8])] = &[
    ("/index.html", "text/html; charset=utf-8", include_bytes!("../web/dist/index.html")),
    ("/app.js", "text/javascript; charset=utf-8", include_bytes!("../web/dist/app.js")),
    ("/app.css", "text/css; charset=utf-8", include_bytes!("../web/dist/app.css")),
    ("/icon.svg", "image/svg+xml", include_bytes!("../web/dist/icon.svg")),
];

pub fn serve(path: &str) -> (u16, &'static str, &'static [u8]) {
    let path = if path.is_empty() || path == "/" { "/index.html" } else { path };
    FILES
        .iter()
        .find(|(p, ..)| *p == path)
        .map_or((404, "text/plain; charset=utf-8", b"not found"), |(_, kind, body)| (200, *kind, *body))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_page_and_its_files_are_built_in() {
        let (status, kind, body) = super::serve("/");
        assert_eq!((status, kind), (200, "text/html; charset=utf-8"));
        assert!(String::from_utf8_lossy(body).contains("app.js"));
        assert_eq!(super::serve("/app.js").0, 200);
        assert_eq!(super::serve("/../secret").0, 404);
    }
}

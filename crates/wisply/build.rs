fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/wisply.ico")
        .set("ProductName", "Wisply")
        .set("FileDescription", "Wisply")
        .set("CompanyName", "QDHShamiro")
        .set("LegalCopyright", "Copyright 2026 QDHShamiro, Apache-2.0");
    if let Err(e) = res.compile() {
        println!("cargo:warning=no Windows version resource: {e}");
    }
}

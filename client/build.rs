use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn rc_exe() -> Option<PathBuf> {
    if let Some(rc) = env::var_os("RC") {
        return Some(PathBuf::from(rc));
    }
    let arch = if env::var("HOST").unwrap_or_default().starts_with("aarch64") { "arm64" } else { "x64" };
    let kits = PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let mut found: Vec<PathBuf> = fs::read_dir(kits)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path().join(arch).join("rc.exe")))
        .filter(|p| p.exists())
        .collect();
    found.sort();
    found.pop()
}

fn main() {
    println!("cargo:rerun-if-changed=lighting.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") || env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    let Some(rc) = rc_exe() else {
        println!("cargo:warning=rc.exe not found, lighting.exe is built without its icon");
        return;
    };
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let ico = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("lighting.ico");
    let script = out.join("lighting.rc");
    let res = out.join("lighting.res");
    fs::write(&script, format!("1 ICON \"{}\"\n", ico.display().to_string().replace('\\', "\\\\"))).unwrap();
    let ok = Command::new(rc).arg("/nologo").arg("/fo").arg(&res).arg(script).status().map(|s| s.success()).unwrap_or(false);
    if ok {
        println!("cargo:rustc-link-arg-bins={}", res.display());
    } else {
        println!("cargo:warning=rc.exe failed, lighting.exe is built without its icon");
    }
}

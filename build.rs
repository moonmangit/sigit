//! Windows: embed the app icon and version details in sigit.exe, so Explorer, the taskbar and
//! Alt+Tab show sigit's icon (the window itself sets none). Other targets need nothing here: the
//! macOS bundle gets its icon from the release workflow.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "sigit");
        res.set("FileDescription", "sigit");
        res.set("LegalCopyright", "MIT License, sigit contributors");
        if let Err(e) = res.compile() {
            // A missing resource compiler shouldn't stop a build: the app works without an icon.
            println!("cargo:warning=no icon embedded: {e}");
        }
    }
}

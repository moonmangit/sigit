//! Windows: embed the app icon and version details in sigit.exe, so Explorer, the taskbar and
//! Alt+Tab show sigit's icon (the window itself sets none). Other targets need nothing here: the
//! macOS bundle gets its icon from the release workflow.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    // The release channel and version (src/channel.rs) are read at build time.
    println!("cargo:rerun-if-env-changed=SIGIT_CHANNEL");
    println!("cargo:rerun-if-env-changed=SIGIT_VERSION");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        let name = if std::env::var("SIGIT_CHANNEL").as_deref() == Ok("beta") { "sigit beta" } else { "sigit" };
        res.set("ProductName", name);
        res.set("FileDescription", name);
        res.set("LegalCopyright", "MIT License, sigit contributors");
        if let Err(e) = res.compile() {
            // A missing resource compiler shouldn't stop a build: the app works without an icon.
            println!("cargo:warning=no icon embedded: {e}");
        }
    }
}

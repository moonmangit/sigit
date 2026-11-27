//! The release channel, fixed when the app is built: stable (the default) or beta, built with
//! `SIGIT_CHANNEL=beta`. A beta installs beside the stable app: its own name, app id, settings
//! folder and download files, and it updates to newer betas (GitHub pre-releases) as well as
//! releases. `SIGIT_VERSION` (the release workflow sets it from the tag) overrides the version
//! in Cargo.toml, so a beta knows it is `0.4.0-beta.1`.

pub fn beta() -> bool {
    matches!(option_env!("SIGIT_CHANNEL"), Some("beta"))
}

/// What people see: the window title, the about page.
pub fn name() -> &'static str {
    if beta() { "sigit beta" } else { "sigit" }
}

/// For folders and file names: the settings folder, the release files, the macOS bundle.
pub fn slug() -> &'static str {
    if beta() { "sigit-beta" } else { "sigit" }
}

/// The OS-level identity (notifications on Windows and macOS).
pub fn app_id() -> &'static str {
    if beta() { "dev.sigit.beta" } else { "dev.sigit.app" }
}

pub fn version() -> &'static str {
    match option_env!("SIGIT_VERSION") {
        Some(v) if !v.trim().is_empty() => v,
        _ => env!("CARGO_PKG_VERSION"),
    }
}

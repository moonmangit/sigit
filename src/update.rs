//! Updates (Settings → About): check the project's public GitHub releases for a newer version,
//! and install it in place with one click.
//!
//! Installing downloads this system's file from the release (the same zip offered for manual
//! download) with `gh`, checks its SHA-256 against the release's `SHA256SUMS.txt`, unpacks it
//! with the system's own tools, and swaps it in for this copy: on Windows the running .exe is
//! renamed aside (Windows allows that) and the new one put in its place; on macOS the whole
//! sigit.app is replaced. Any failure puts the old copy back. The new version runs after a
//! restart; the copy set aside is deleted on that launch.

use std::path::{Path, PathBuf};

use crate::{State, gh, jobs};

/// The repository releases come from: `repository` in Cargo.toml.
pub fn repo() -> &'static str {
    env!("CARGO_PKG_REPOSITORY").trim_start_matches("https://github.com/").trim_end_matches('/')
}

pub fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Where updating got to.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Update {
    #[default]
    NotChecked,
    Checking,
    /// This build is the latest release (or newer).
    UpToDate,
    /// A newer release: its tag, version and page.
    Available { tag: String, version: String, url: String },
    /// Downloading, checking and swapping it in.
    Installing { version: String },
    /// In place: the next start runs it.
    Installed { version: String },
    Failed(String),
}

/// "v1.2.3" or "1.2.3" → (1, 2, 3); anything after a '-' or '+' is ignored.
pub fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let core = v.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((parts.next()??, parts.next().unwrap_or(Some(0))?, parts.next().unwrap_or(Some(0))?))
}

/// The latest published release (not a draft or pre-release): (tag, page).
fn latest() -> Result<(String, String), String> {
    let out = check_cmd().run(None).map_err(|e| if e.contains("404") || e.contains("Not Found") { "No releases published yet".to_owned() } else { e })?;
    let v: serde_json::Value = serde_json::from_str(&out).map_err(|e| e.to_string())?;
    let tag = v["tag_name"].as_str().ok_or("unexpected answer from GitHub")?.to_owned();
    let url = v["html_url"].as_str().unwrap_or_default().to_owned();
    Ok((tag, url))
}

/// Compare a release tag with this build.
pub fn verdict(tag: &str, url: String) -> Update {
    match (parse(tag), parse(current())) {
        (Some(latest), Some(mine)) if latest > mine => Update::Available { tag: tag.to_owned(), version: tag.trim_start_matches('v').to_owned(), url },
        (Some(_), Some(_)) => Update::UpToDate,
        _ => Update::Failed(tf!("Couldn't read the version \"{tag}\"")),
    }
}

// ---------------------------------------------------------------------------------------------
// Installing
// ---------------------------------------------------------------------------------------------

/// The release file for this system, if the releases have one.
/// Asks GitHub for the newest release.
pub fn check_cmd() -> gh::Cmd {
    gh::Cmd::new("gh", &["api", &format!("repos/{}/releases/latest", repo())])
}

/// Downloads this system's build of `tag` and the checksums into `dir`.
pub fn download_cmd(tag: &str, asset: &str, dir: &str) -> gh::Cmd {
    gh::Cmd::new("gh", &["release", "download", tag, "--repo", repo(), "--pattern", asset, "--pattern", "SHA256SUMS.txt", "--dir", dir, "--clobber"])
}

pub fn asset() -> Option<&'static str> {
    if cfg!(all(windows, target_arch = "x86_64")) {
        Some("sigit-windows-x64.zip")
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Some("sigit-macos-arm64.zip")
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Some("sigit-macos-x64.zip")
    } else {
        None
    }
}

/// Why this copy can't update itself, if it can't (the button then offers the release page).
pub fn cannot_install() -> Option<String> {
    if asset().is_none() {
        return Some("There's no ready-made build for this system: update from source.".into());
    }
    target().err()
}

/// What an update replaces: this .exe, or on macOS the sigit.app it runs in.
fn target() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if exe.components().any(|c| c.as_os_str() == "target") {
        return Err("This is a development build (run from target/): update it from source.".into());
    }
    if cfg!(target_os = "macos")
        && let Some(app) = exe.ancestors().nth(3).filter(|p| p.extension().is_some_and(|e| e == "app"))
    {
        return Ok(app.to_path_buf());
    }
    Ok(exe)
}

/// Where the replaced copy is set aside until the next start.
fn set_aside(target: &Path) -> PathBuf {
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match name.strip_suffix(".exe") {
        Some(stem) => target.with_file_name(format!("{stem}.old.exe")),
        None => target.with_file_name(format!("{name}.old")),
    }
}

/// Download, check and swap in release `tag`. Returns what to start to run it.
pub fn install(tag: &str) -> Result<PathBuf, String> {
    let asset = asset().ok_or("There's no ready-made build for this system")?;
    let target = target()?;
    let work = std::env::temp_dir().join(format!("sigit-update-{}", tag.replace(['/', '\\'], "_")));
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let dir = work.to_string_lossy().into_owned();
    download_cmd(tag, asset, &dir)
        .run(None)
        .map_err(|e| tf!("Download failed: {e}"))?;
    let zip = work.join(asset);
    let sums = std::fs::read_to_string(work.join("SHA256SUMS.txt")).map_err(|_| "The release has no checksums".to_owned())?;
    let bytes = std::fs::read(&zip).map_err(|e| e.to_string())?;
    verify(asset, &bytes, &sums)?;
    let unpacked = work.join("unpacked");
    unzip(&zip, &unpacked)?;
    replace(&target, &unpacked)?;
    let _ = std::fs::remove_dir_all(&work);
    Ok(target)
}

/// The file's SHA-256 must match its line in SHA256SUMS.txt.
pub fn verify(name: &str, bytes: &[u8], sums: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    let want = sums
        .lines()
        .find_map(|l| {
            let (hash, file) = l.trim().split_once(char::is_whitespace)?;
            (file.trim().trim_start_matches('*') == name).then(|| hash.to_ascii_lowercase())
        })
        .ok_or_else(|| tf!("{name} isn't in the release's checksums"))?;
    let got: String = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
    if got == want { Ok(()) } else { Err("The download doesn't match its checksum, so it wasn't installed".into()) }
}

fn unzip(zip: &Path, out: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let q = |p: &Path| p.to_string_lossy().replace('\'', "''");
        let cmd = format!("Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force", q(zip), q(out));
        gh::run("powershell", &["-NoProfile", "-NonInteractive", "-Command", &cmd], None).map(|_| ())
    }
    #[cfg(not(windows))]
    {
        gh::run("ditto", &["-x", "-k", &zip.to_string_lossy(), &out.to_string_lossy()], None).map(|_| ())
    }
}

/// Swap the unpacked new copy in for `target`, keeping the old one aside (and putting it back
/// if anything fails).
fn replace(target: &Path, unpacked: &Path) -> Result<(), String> {
    let new = if target.extension().is_some_and(|e| e == "app") {
        unpacked.join("sigit.app")
    } else if cfg!(windows) {
        unpacked.join("sigit.exe")
    } else {
        unpacked.join("sigit.app").join("Contents").join("MacOS").join("sigit")
    };
    if !new.exists() {
        return Err("The download didn't contain sigit".into());
    }
    let old = set_aside(target);
    remove(&old);
    match std::fs::rename(target, &old) {
        Ok(()) => {}
        // Installed where this account can't write (Program Files for all users, or
        // /Applications for a standard Mac account): do the swap with the system's permission.
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return swap_as_admin(target, &new, &old),
        Err(e) => return Err(tf!("Couldn't move the current version aside: {e}")),
    }
    if let Err(e) = copy_into_place(&new, target) {
        remove(target);
        let _ = std::fs::rename(&old, target);
        return Err(tf!("Couldn't put the new version in place: {e}"));
    }
    Ok(())
}

/// The same swap as `replace` (set the old copy aside, put the new one in place, put the old
/// one back if that fails), run with administrator rights. Windows shows its permission
/// prompt (UAC); macOS asks for an administrator's password.
#[cfg(windows)]
fn swap_as_admin(target: &Path, new: &Path, old: &Path) -> Result<(), String> {
    let script = elevated_swap_script(target, new, old);
    // An outer PowerShell starts the elevated one and waits for it; the swap travels encoded,
    // so no path needs quoting on a command line.
    let outer = format!(
        "$p = Start-Process powershell -Verb RunAs -Wait -PassThru -WindowStyle Hidden -ArgumentList '-NoProfile','-NonInteractive','-EncodedCommand','{}'; exit $p.ExitCode",
        encode_command(&script)
    );
    match gh::run("powershell", &["-NoProfile", "-NonInteractive", "-Command", &outer], None) {
        Ok(_) => Ok(()),
        Err(e) if e.to_lowercase().contains("canceled") || e.to_lowercase().contains("cancelled") => {
            Err("sigit is installed for all users, so updating needs administrator permission, which wasn't given".into())
        }
        Err(e) => Err(tf!("Couldn't replace sigit even with administrator permission: {e}")),
    }
}

/// PowerShell that swaps `new` in for `target`, keeping `target` aside as `old` and putting it
/// back if the copy fails. Exit 0 on success.
#[cfg(windows)]
fn elevated_swap_script(target: &Path, new: &Path, old: &Path) -> String {
    let q = |p: &Path| p.to_string_lossy().replace('\'', "''");
    let (t, n, o) = (q(target), q(new), q(old));
    format!(
        "$ErrorActionPreference = 'Stop'\n\
         try {{\n\
           if (Test-Path -LiteralPath '{o}') {{ Remove-Item -LiteralPath '{o}' -Force }}\n\
           Move-Item -LiteralPath '{t}' -Destination '{o}'\n\
           try {{ Copy-Item -LiteralPath '{n}' -Destination '{t}' }}\n\
           catch {{ Move-Item -LiteralPath '{o}' -Destination '{t}' -Force; exit 2 }}\n\
           exit 0\n\
         }} catch {{ exit 1 }}\n"
    )
}

/// PowerShell's -EncodedCommand: base64 of the script in UTF-16LE.
#[cfg(windows)]
fn encode_command(script: &str) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ABC[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(not(windows))]
fn swap_as_admin(target: &Path, new: &Path, old: &Path) -> Result<(), String> {
    // A shell command run with an administrator's password through AppleScript.
    let sh = |p: &Path| format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"));
    let (t, n, o) = (sh(target), sh(new), sh(old));
    let cmd = format!("rm -rf {o}; mv {t} {o} && (ditto {n} {t} || (rm -rf {t}; mv {o} {t}; exit 2)) && touch {t}");
    let script = format!("do shell script \"{}\" with administrator privileges", cmd.replace('\\', "\\\\").replace('"', "\\\""));
    match gh::run("osascript", &["-e", &script], None) {
        Ok(_) => Ok(()),
        Err(e) if e.contains("-128") || e.to_lowercase().contains("cancel") => Err("Updating needs an administrator's password, which wasn't given".into()),
        Err(e) => Err(tf!("Couldn't replace sigit even with administrator permission: {e}")),
    }
}

fn copy_into_place(new: &Path, target: &Path) -> Result<(), String> {
    if new.is_dir() {
        // An .app bundle: ditto copies it whole (signature, permissions, icon) across volumes.
        gh::run("ditto", &[&new.to_string_lossy(), &target.to_string_lossy()], None)?;
        // Finder and the Dock cache app icons by path: a newer modification date makes them
        // read the new bundle's icon instead of the old copy's.
        let _ = gh::run("touch", &[&target.to_string_lossy()], None);
        Ok(())
    } else {
        std::fs::copy(new, target).map(|_| ()).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o755));
        }
        Ok(())
    }
}

fn remove(p: &Path) {
    if p.is_dir() {
        let _ = std::fs::remove_dir_all(p);
    } else {
        let _ = std::fs::remove_file(p);
    }
}

/// Start the installed copy and quit this one.
pub fn restart(target: &Path) -> ! {
    let started = if target.extension().is_some_and(|e| e == "app") {
        std::process::Command::new("open").arg("-n").arg(target).spawn()
    } else {
        std::process::Command::new(target).spawn()
    };
    if let Err(e) = started {
        eprintln!("couldn't start {}: {e}", target.display());
    }
    std::process::exit(0)
}

/// On start: delete the copy an update set aside.
pub fn clean_up_previous() {
    if let Ok(target) = target() {
        remove(&set_aside(&target));
    }
}

impl State {
    pub fn check_for_updates(&mut self) {
        if matches!(self.update, Update::Checking | Update::Installing { .. }) {
            return;
        }
        self.update = Update::Checking;
        jobs::spawn(self, latest, |s, r| {
            s.update = match r {
                Ok((tag, url)) => verdict(&tag, url),
                Err(e) => Update::Failed(e),
            };
        });
    }

    /// Download and swap in the available version.
    pub fn install_update(&mut self) {
        let Update::Available { tag, version, .. } = self.update.clone() else { return };
        self.update = Update::Installing { version: version.clone() };
        jobs::spawn(self, move || install(&tag), move |s, r| {
            s.update = match r {
                Ok(_) => Update::Installed { version },
                Err(e) => Update::Failed(e),
            };
        });
    }

    /// Run the installed version.
    pub fn restart_after_update(&mut self) {
        if let Ok(target) = target() {
            self.save_prefs();
            restart(&target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        assert_eq!(parse("v0.1.0"), Some((0, 1, 0)));
        assert_eq!(parse("1.2"), Some((1, 2, 0)));
        assert_eq!(parse("v2.0.1-beta.1"), Some((2, 0, 1)));
        assert_eq!(parse("latest"), None);
    }

    #[test]
    fn newer_only_when_greater() {
        let url = String::from("https://example.test/r");
        assert_eq!(verdict(&format!("v{}", current()), url.clone()), Update::UpToDate);
        assert!(matches!(verdict("v999.0.0", url.clone()), Update::Available { ref version, ref tag, .. } if version == "999.0.0" && tag == "v999.0.0"));
        assert_eq!(verdict("v0.0.1", url.clone()), Update::UpToDate);
        assert!(matches!(verdict("nightly", url), Update::Failed(_)));
        assert!(!repo().is_empty() && !repo().starts_with("https://"));
    }

    #[test]
    fn checksums_must_match() {
        // sha256("hello")
        let sums = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824  sigit-windows-x64.zip\nabc  other.zip\n";
        assert!(verify("sigit-windows-x64.zip", b"hello", sums).is_ok());
        assert!(verify("sigit-windows-x64.zip", b"tampered", sums).is_err());
        assert!(verify("missing.zip", b"hello", sums).is_err());
    }

    #[test]
    fn old_copy_names() {
        assert_eq!(set_aside(Path::new("C:/apps/sigit.exe")), Path::new("C:/apps/sigit.old.exe"));
        assert_eq!(set_aside(Path::new("/Applications/sigit.app")), Path::new("/Applications/sigit.app.old"));
    }

    #[cfg(windows)]
    #[test]
    fn encodes_like_powershell() {
        // [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes("exit 0"))
        assert_eq!(encode_command("exit 0"), "ZQB4AGkAdAAgADAA");
    }

    /// The elevated swap's script, run here without elevation on throwaway files.
    #[cfg(windows)]
    #[test]
    fn elevated_swap_script_swaps_and_keeps_the_old_copy() {
        let dir = std::env::temp_dir().join(format!("sigit-swap-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("it's new")).unwrap();
        let (target, new, old) = (dir.join("sigit.exe"), dir.join("it's new").join("sigit.exe"), dir.join("sigit.old.exe"));
        std::fs::write(&target, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        let script = elevated_swap_script(&target, &new, &old);
        let out = std::process::Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encode_command(&script)]).status().unwrap();
        assert!(out.success());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "old");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dev_builds_dont_replace_themselves() {
        // Tests run from target/, so this is a development build.
        assert!(cannot_install().is_some());
    }
}

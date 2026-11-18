//! Errors that would close the app: kept, shown, and survived.
//!
//! Every panic, on any thread, becomes a [`Report`]: written to `%APPDATA%\sigit\sigit.log`
//! (`~/.config/sigit/` elsewhere) and queued for the error dialog, which has a Copy button.
//! - A panic in the view, `poll`, a key or a background job's result is caught where Rune calls
//!   into sigit (`main.rs`, `jobs.rs`): the app keeps running and shows the dialog.
//! - One that can't be caught there (inside Rune, or in a button's handler) ends the event loop.
//!   `main` then saves the report and starts sigit again, which shows it on launch. Twice within
//!   a few seconds and it stops instead, with a plain system message box.
//!
//! A release build on Windows has no console (`windows_subsystem = "windows"`), so stderr is
//! pointed at the log too: Rune's own warnings ("the GPU device was lost") land there as well.

use std::io::Write;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One error, for the dialog: `summary` is what it shows, `details` what Copy copies.
#[derive(Clone, Debug, PartialEq)]
pub struct Report {
    pub summary: String,
    pub details: String,
}

/// Reports waiting for `poll` to show them.
static PENDING: Mutex<Vec<Report>> = Mutex::new(Vec::new());
/// The newest report, for the view's own fallback screen (which can't take from `PENDING`).
static LATEST: Mutex<Option<Report>> = Mutex::new(None);
/// The last panic's message and place, and when it last fired: a view that panics on every
/// frame logs and reports once, not sixty times a second.
static REPEAT: Mutex<Option<(String, Instant)>> = Mutex::new(None);
const REPEAT_QUIET: Duration = Duration::from_secs(3);
/// Set on the copy `restart` starts, with the time it started: a crash soon after means the
/// error comes back on its own, so don't restart again.
const RESTARTED: &str = "SIGIT_RESTARTED";
const RESTART_LOOP_SECS: u64 = 20;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn install() {
    #[cfg(all(windows, not(debug_assertions)))]
    redirect_stderr();
    if let Some(r) = load_previous() {
        push(r);
    }
    std::panic::set_hook(Box::new(|info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".to_owned());
        let at = info.location().map_or_else(String::new, |l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let key = format!("{msg}\n{at}");
        {
            let mut last = lock(&REPEAT);
            let repeat = last.as_ref().is_some_and(|(k, t)| *k == key && t.elapsed() < REPEAT_QUIET);
            *last = Some((key, Instant::now()));
            if repeat {
                return;
            }
        }
        let when = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let thread = std::thread::current().name().unwrap_or("unnamed").to_owned();
        let summary = format!("{msg}\n  at {at}");
        let details = format!(
            "sigit {} on {} panicked on thread '{thread}' at {when}\n{summary}\n\nBacktrace:\n{}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::backtrace::Backtrace::force_capture()
        );
        eprintln!("\n--- {details}");
        let _ = std::io::stderr().flush();
        push(Report { summary, details });
        // Wake the UI so `poll` shows it, also when the panic was on a background thread.
        crate::jobs::update(|_| {});
    }));
}

fn push(r: Report) {
    *lock(&LATEST) = Some(r.clone());
    lock(&PENDING).push(r);
}

/// The reports since the last call, oldest first.
pub fn take() -> Vec<Report> {
    std::mem::take(&mut *lock(&PENDING))
}

/// The newest report, if any.
pub fn latest() -> Option<Report> {
    lock(&LATEST).clone()
}

/// Runs `f`, turning a panic into `None` (the hook has already reported it).
pub fn catch<R>(f: impl FnOnce() -> R) -> Option<R> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

fn previous_path() -> Option<std::path::PathBuf> {
    crate::config::dir().map(|d| d.join("last-crash.txt"))
}

/// The error that closed the last run, once: shown on this launch, then forgotten.
fn load_previous() -> Option<Report> {
    let path = previous_path()?;
    let details = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let summary = details.lines().skip(1).take_while(|l| !l.is_empty()).collect::<Vec<_>>().join("\n");
    Some(Report { summary: format!("sigit closed after this error and restarted:\n{summary}"), details })
}

/// After an error the event loop couldn't survive: keep the report for the next launch and
/// start sigit again, unless this copy was itself just restarted.
pub fn restart() {
    let Some(report) = latest() else { return };
    if let Some(path) = previous_path() {
        let _ = std::fs::write(path, &report.details);
    }
    let now = crate::model::now() as u64;
    let looping = std::env::var(RESTARTED).ok().and_then(|t| t.parse::<u64>().ok()).is_some_and(|t| now.saturating_sub(t) < RESTART_LOOP_SECS);
    if !looping {
        if let Ok(exe) = std::env::current_exe() {
            if std::process::Command::new(exe).args(std::env::args_os().skip(1)).env(RESTARTED, now.to_string()).spawn().is_ok() {
                return;
            }
        }
    }
    give_up(&report);
}

/// The error keeps coming back: say so with the OS's own message box (Ctrl+C copies its text).
fn give_up(report: &Report) {
    let text = format!("sigit hit an error it couldn't recover from and closed.\n\n{}\n\nThe full report is in sigit.log in {}.", report.summary, crate::config::dir().map_or_else(String::new, |d| d.display().to_string()));
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
        let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
        // SAFETY: both strings are NUL-terminated and outlive the call.
        unsafe { MessageBoxW(std::ptr::null_mut(), wide(&text).as_ptr(), wide("sigit").as_ptr(), MB_OK | MB_ICONERROR) };
    }
    #[cfg(not(windows))]
    eprintln!("{text}");
}

#[cfg(all(windows, not(debug_assertions)))]
fn redirect_stderr() {
    use std::os::windows::io::IntoRawHandle;
    use windows_sys::Win32::System::Console::{STD_ERROR_HANDLE, SetStdHandle};
    // The log grows across runs (a crash must survive the relaunch after it); past 1 MiB it
    // starts over.
    const MAX_BYTES: u64 = 1 << 20;
    let Some(path) = crate::config::dir().map(|d| d.join("sigit.log")) else { return };
    let _ = std::fs::create_dir_all(path.parent().unwrap_or(&path));
    let too_big = std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES);
    let file = std::fs::OpenOptions::new().create(true).append(!too_big).write(true).truncate(too_big).open(&path);
    let Ok(file) = file else { return };
    // Rust's stderr looks the handle up on every write, so from here on eprintln!, the panic
    // report and Rune's warnings land in the file. The handle stays open for the whole run.
    // SAFETY: hands the process a valid, open file handle that is never closed.
    unsafe { SetStdHandle(STD_ERROR_HANDLE, file.into_raw_handle() as _) };
}

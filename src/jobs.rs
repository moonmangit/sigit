//! Background work. `spawn` runs a blocking `gh`/`git` call on its own thread and hands the
//! result to a closure that runs on the UI thread with `&mut State`, through Rune's
//! `AppHandle::update`. The UI never waits on the network.
//!
//! A panic in a result's closure is caught (`crash_log`): the error dialog shows it and the app
//! keeps running.
//!
//! Headless runs (`--snap`, tests) have no event loop to deliver results into, so there the
//! work runs inline and the result lands before the frame is drawn.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rune::AppHandle;

use crate::State;

static HANDLE: OnceLock<AppHandle<State>> = OnceLock::new();
static INLINE: AtomicBool = AtomicBool::new(true);

/// Called once from `main` with the app's handle; from then on work runs in the background.
pub fn install(handle: AppHandle<State>) {
    let _ = HANDLE.set(handle);
    INLINE.store(false, Ordering::Relaxed);
}

/// Search-as-you-type: wait `delay`, and run `work` only if no newer request came in meanwhile
/// (each call bumps `latest`); an answer that lands after a newer request started is dropped.
/// So typing "release" runs one search, for "release", not seven.
pub fn debounced<T: Send + 'static>(
    s: &mut State,
    latest: &Arc<AtomicU64>,
    delay: Duration,
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(&mut State, T) + Send + 'static,
) {
    let me = latest.fetch_add(1, Ordering::SeqCst) + 1;
    match HANDLE.get().filter(|_| !INLINE.load(Ordering::Relaxed)) {
        Some(h) => {
            let (h, latest) = (h.clone(), latest.clone());
            std::thread::spawn(move || {
                std::thread::sleep(delay);
                if latest.load(Ordering::SeqCst) != me {
                    return; // superseded while waiting: never ran
                }
                let out = work();
                h.update(move |s, _| {
                    if latest.load(Ordering::SeqCst) == me {
                        crate::crash_log::catch(|| done(s, out));
                    }
                });
            });
        }
        None => {
            let out = work();
            done(s, out);
        }
    }
}

/// Forget pending debounced work (the box was cleared).
pub fn cancel(latest: &Arc<AtomicU64>) {
    latest.fetch_add(1, Ordering::SeqCst);
}

pub fn spawn<T: Send + 'static>(
    s: &mut State,
    work: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(&mut State, T) + Send + 'static,
) {
    match HANDLE.get().filter(|_| !INLINE.load(Ordering::Relaxed)) {
        Some(h) => {
            let h = h.clone();
            std::thread::spawn(move || {
                let out = work();
                h.update(move |s, _| {
                    crate::crash_log::catch(|| done(s, out));
                });
            });
        }
        None => {
            let out = work();
            done(s, out);
        }
    }
}

/// Run each of `works` on its own thread and hand every result to `done` on the UI thread as
/// soon as it's ready, so a page fills in piece by piece instead of waiting for the slowest.
/// `done` returns true when its change should animate (Rune lands data a server brought at
/// once unless asked).
pub fn spawn_each<T: Send + 'static>(s: &mut State, works: Vec<Box<dyn FnOnce() -> T + Send>>, done: impl Fn(&mut State, T) -> bool + Send + Sync + 'static) {
    let done = Arc::new(done);
    match HANDLE.get().filter(|_| !INLINE.load(Ordering::Relaxed)) {
        Some(h) => {
            for work in works {
                let (h, done) = (h.clone(), done.clone());
                std::thread::spawn(move || {
                    let out = work();
                    h.update(move |s, tx| {
                        if crate::crash_log::catch(|| done(s, out)).unwrap_or(false) {
                            tx.animated();
                        }
                    });
                });
            }
        }
        None => {
            for work in works {
                let out = work();
                let _ = done(s, out);
            }
        }
    }
}

/// Headless runs (`--snap`, `--tree`, tests) work inline.
pub fn is_inline_run() -> bool {
    INLINE.load(Ordering::Relaxed)
}

/// Change state from any thread (a notification click).
pub fn update(f: impl FnOnce(&mut State) + Send + 'static) {
    if let Some(h) = HANDLE.get() {
        h.update(move |s, _| {
            crate::crash_log::catch(|| f(s));
        });
    }
}

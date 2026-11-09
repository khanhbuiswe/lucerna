//! Panic isolation for background work: a panic in a load, parse, search
//! or watcher is logged and contained instead of taking the app down.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::thread::{self, JoinHandle};

/// Runs `f` on its own thread named `name`. A panic is logged with
/// `tracing::error!(task = name, …)` and the thread yields `None`.
///
/// # Panics
/// If the OS cannot create a thread, as [`std::thread::spawn`] does; use
/// [`try_spawn_guarded`] to handle that.
pub fn spawn_guarded<T: Send + 'static>(
    name: &'static str,
    f: impl FnOnce() -> T + Send + 'static,
) -> JoinHandle<Option<T>> {
    try_spawn_guarded(name, f).unwrap_or_else(|err| panic!("failed to spawn thread {name}: {err}"))
}

/// Like [`spawn_guarded`], returning the error when the OS cannot create
/// the thread.
///
/// # Errors
/// When the thread cannot be created.
pub fn try_spawn_guarded<T: Send + 'static>(
    name: &'static str,
    f: impl FnOnce() -> T + Send + 'static,
) -> std::io::Result<JoinHandle<Option<T>>> {
    thread::Builder::new()
        .name(name.into())
        .spawn(move || guarded(name, f))
}

/// Runs `f` here, for closures on the GPUI background executor: a panic is
/// logged and becomes `None`.
pub fn guarded<T>(name: &'static str, f: impl FnOnce() -> T) -> Option<T> {
    catch_panic(name, f).ok()
}

/// Like [`guarded`], keeping the panic message for callers that report it.
pub fn catch_panic<T>(name: &'static str, f: impl FnOnce() -> T) -> Result<T, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|panic| {
        let message = panic_message(panic.as_ref()).to_owned();
        tracing::error!(task = name, message, "background task panicked");
        message
    })
}

fn panic_message(panic: &(dyn Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic")
}

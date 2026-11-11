//! Panic containment for transport threads (same behaviour as the
//! workbench's `tasks` module, which this crate cannot depend on).

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Runs `f`; a panic is logged with `tracing::error!(task = name, …)` and
/// reported as `Err`.
pub(crate) fn catch_panic<T>(name: &str, f: impl FnOnce() -> T) -> Result<T, ()> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|panic| {
        let message = panic_message(panic.as_ref());
        tracing::error!(task = name, message, "lsp transport thread panicked");
    })
}

fn panic_message(panic: &(dyn Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic")
}

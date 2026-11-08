//! Rope-backed text buffer: load with encoding detection, cheap snapshots,
//! and reload diffing so scroll position survives an external rewrite.

mod buffer;
mod load;
mod reload;

pub use buffer::{Buffer, BufferSnapshot, LineEnding, Point};
pub use load::{LoadError, MAX_OPEN_BYTES, check_size};
pub use reload::Reload;

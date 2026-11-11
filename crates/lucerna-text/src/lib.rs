//! Rope-backed text buffer: load with encoding detection, cheap snapshots,
//! and reload diffing so scroll position survives an external rewrite.

mod buffer;
mod load;
mod patch;
mod reload;

pub use buffer::{Buffer, BufferSnapshot, LineEnding, Point};
pub use load::{Decoded, LoadError, MAX_OPEN_BYTES, check_size, decode_file};
pub use patch::{Bias, Patch, PatchEdit};
pub use reload::Reload;

//! Rope-backed text buffer: load with encoding detection, cheap snapshots,
//! edits with undo, and line diffs so a reload is an edit like any other.

mod buffer;
mod diff;
mod history;
mod load;
mod patch;
mod reload;
pub mod words;
mod write;

pub use buffer::{Buffer, BufferSnapshot, LineEnding, Point};
pub use diff::diff_edits;
pub use history::{HistoryState, SelectionsToken, TransactionId, UndoResult};
pub use load::{Decoded, LoadError, MAX_OPEN_BYTES, check_regular, check_size, decode_file};
pub use patch::{Bias, Patch, PatchEdit};
pub use reload::Reload;
pub use write::{TempFiles, WriteError, Written, write_bytes, write_bytes_tracked};

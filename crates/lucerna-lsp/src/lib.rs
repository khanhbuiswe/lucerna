//! Language server client for Lucerna. No gpui, no async runtime.

pub mod capabilities;
pub mod client;
pub mod framing;
mod guard;
pub mod position;
pub mod process;
#[cfg(feature = "test-support")]
pub mod test_support;
pub mod transport;

pub use capabilities::initialize_params;
pub use client::{LanguageServer, LogKind, LogLine, LspError, RequestFuture, ServerEvent};

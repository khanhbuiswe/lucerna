//! Language server client for Lucerna. No gpui, no async runtime.

pub mod adapters;
pub mod capabilities;
pub mod client;
pub mod diagnostics;
pub mod env;
pub mod framing;
mod guard;
pub mod health;
pub mod launcher;
pub mod locations;
pub mod position;
pub mod process;
pub mod registry;
pub mod relay;
pub mod settings;
pub mod symbols;
pub mod sync;
pub mod table;
#[cfg(feature = "test-support")]
pub mod test_support;
pub mod transport;
pub mod vars;

pub use adapters::{RootSearch, find_root};
pub use capabilities::initialize_params;
pub use client::{LanguageServer, LogKind, LogLine, LspError, RequestFuture, ServerEvent};
pub use launcher::{Launcher, ProcessLauncher};
pub use registry::{Registry, ServerKey, SettingsDiff, Slot};
pub use settings::{BinarySettings, StartError, resolve_launch, select_servers};
pub use table::{ServerDef, ServerTable, TableError};
pub use vars::Vars;

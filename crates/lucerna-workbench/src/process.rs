//! Starting another `luc` process: a process shows one window, so a folder
//! opened in a new window, or a new Welcome window, runs in its own.

use std::io;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

use gpui::{App, Global};

/// What a new `luc` process opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnRequest {
    /// `luc <path>`.
    Path(PathBuf),
    /// `luc` with no path, started in `/` as Finder starts the app, so it
    /// shows the Welcome screen (or the folder prompt).
    Welcome,
}

/// Starts the process a [`SpawnRequest`] describes.
pub type SpawnFn = dyn Fn(&SpawnRequest) -> io::Result<()>;

/// Starts the processes [`spawn`] asks for; the app sets the real one,
/// tests a fake that records the requests.
#[derive(Clone)]
pub struct Spawner(pub Rc<SpawnFn>);

impl Default for Spawner {
    fn default() -> Self {
        Spawner(Rc::new(real_spawn))
    }
}

impl Global for Spawner {}

/// Starts the process `request` describes, through the [`Spawner`] global
/// (the real spawner when none is set).
pub fn spawn(request: &SpawnRequest, cx: &App) -> io::Result<()> {
    match cx.try_global::<Spawner>() {
        Some(spawner) => (spawner.0)(request),
        None => real_spawn(request),
    }
}

/// Runs this executable as [`SpawnRequest`] says and reaps it on a thread
/// when it exits, so it does not linger as a zombie.
fn real_spawn(request: &SpawnRequest) -> io::Result<()> {
    let mut command = Command::new(std::env::current_exe()?);
    match request {
        SpawnRequest::Path(path) => command.arg(path),
        SpawnRequest::Welcome => command.current_dir("/"),
    };
    let mut child = command.spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

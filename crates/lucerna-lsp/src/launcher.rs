//! Finding and starting servers, behind a trait so tests can fake it.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use crate::env::{login_shell_env, which};
use crate::process::spawn_process;
use crate::transport::{Inbox, LaunchSpec, Transport};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

pub trait Launcher: Send + Sync + 'static {
    /// The environment servers run in. May block on the first call; cached
    /// afterwards.
    fn env(&self) -> HashMap<String, String>;
    /// The executable `name` on this environment's `PATH`.
    fn which(&self, name: &str) -> Option<PathBuf>;
    /// Runs `program args` with stdin closed; true when it exits with
    /// success within 5 s. May block.
    fn probe(&self, program: &Path, args: &[&str]) -> bool;
    /// Starts the server.
    ///
    /// # Errors
    /// When the process cannot be started.
    fn launch(&self, spec: &LaunchSpec, inbox: Arc<dyn Inbox>) -> io::Result<Arc<dyn Transport>>;
}

/// Starts real processes in the user's login-shell environment.
#[derive(Default)]
pub struct ProcessLauncher {
    env: OnceLock<HashMap<String, String>>,
}

impl ProcessLauncher {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Launcher for ProcessLauncher {
    fn env(&self) -> HashMap<String, String> {
        self.env.get_or_init(login_shell_env).clone()
    }

    fn which(&self, name: &str) -> Option<PathBuf> {
        let env = self.env.get_or_init(login_shell_env);
        which(name, env.get("PATH").map_or("", String::as_str))
    }

    fn probe(&self, program: &Path, args: &[&str]) -> bool {
        let Ok(mut child) = Command::new(program)
            .args(args)
            .envs(self.env.get_or_init(login_shell_env))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            return false;
        };
        let deadline = Instant::now() + PROBE_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(5));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
            }
        }
    }

    fn launch(&self, spec: &LaunchSpec, inbox: Arc<dyn Inbox>) -> io::Result<Arc<dyn Transport>> {
        spawn_process(spec, inbox)
    }
}

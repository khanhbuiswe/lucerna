//! Per-extension-id lock shared by every process (app and `luc ext`).

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::store::StoreError;

/// How long a held lock is retried before reporting `Busy`. A child forked
/// by another thread inherits a duplicate of the lock descriptor until it
/// execs, so a just-released lock can look held for a few milliseconds; a
/// real concurrent operation holds it for seconds.
const BUSY_GRACE: Duration = Duration::from_millis(200);

/// Holds `flock(LOCK_EX)` on `<root>/locks/<id>.lock` until dropped.
///
/// `flock` locks belong to the open file description, so a second
/// acquisition fails even from another thread of the same process.
#[derive(Debug)]
pub struct IdLock {
    _file: File,
}

impl IdLock {
    /// Takes the lock without blocking (beyond [`BUSY_GRACE`]);
    /// `StoreError::Busy(id)` when another operation on `id` holds it.
    pub fn try_acquire(root: &Path, id: &str) -> Result<IdLock, StoreError> {
        Self::acquire_within(root, id, BUSY_GRACE)
    }

    /// [`IdLock::try_acquire`] without the grace period: fails with `Busy`
    /// at once. For housekeeping that simply skips busy ids.
    pub fn try_acquire_now(root: &Path, id: &str) -> Result<IdLock, StoreError> {
        Self::acquire_within(root, id, Duration::ZERO)
    }

    fn acquire_within(root: &Path, id: &str, grace: Duration) -> Result<IdLock, StoreError> {
        let dir = root.join("locks");
        let io = |e: std::io::Error| StoreError::Io(format!("lock for `{id}`: {e}"));
        std::fs::create_dir_all(&dir).map_err(io)?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(dir.join(format!("{id}.lock")))
            .map_err(io)?;
        let deadline = Instant::now() + grace;
        loop {
            // SAFETY: `file` is an open descriptor for the duration of the call.
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if rc == 0 {
                return Ok(IdLock { _file: file });
            }
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() != Some(libc::EWOULDBLOCK) {
                return Err(io(err));
            }
            if Instant::now() >= deadline {
                return Err(StoreError::Busy(id.to_string()));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// Blocking `flock(LOCK_EX)` on `<root>/locks/.index.lock`, held while the
/// returned file is open. Serialises index rebuild + write across
/// processes (ids cannot start with `.`, so it never clashes with an id).
pub(crate) fn lock_index(root: &Path) -> std::io::Result<File> {
    lock_named(root, ".index.lock")
}

/// Blocking `flock(LOCK_EX)` on `<root>/locks/.registry.lock`, held while
/// the returned file is open: every read-modify-write of `registry.json`
/// (refresh, describe) runs under it, in this process and across processes.
/// `flock` locks belong to an open file description, so two threads of one
/// process exclude each other too.
pub(crate) fn lock_registry(root: &Path) -> std::io::Result<File> {
    lock_named(root, ".registry.lock")
}

/// Blocking `flock(LOCK_EX)` on `<root>/locks/.registry-clone.lock`, held
/// for a whole refresh of the `registry/` clone (clone or fetch, update-ref,
/// reads), so two refreshes (app and CLI, or two threads) never run git in
/// it at once nor delete it under each other. Separate from
/// [`lock_registry`], which stays short.
pub(crate) fn lock_registry_clone(root: &Path) -> std::io::Result<File> {
    lock_named(root, ".registry-clone.lock")
}

fn lock_named(root: &Path, name: &str) -> std::io::Result<File> {
    let dir = root.join("locks");
    std::fs::create_dir_all(&dir)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(name))?;
    loop {
        // SAFETY: `file` is an open descriptor for the duration of the call.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(file);
        }
        let err = std::io::Error::last_os_error();
        if err.kind() != std::io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
}

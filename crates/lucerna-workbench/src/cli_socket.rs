//! The socket `luc` hands its paths to the running app through (spec M13,
//! §6.5 of the multi-window design): one line of JSON each way.
//!
//! `luc` [`claim`]s the socket before anything else. Whoever binds it is
//! the app and passes the listener to `app::run`, which [`serve`]s it on a
//! thread; a later `luc` finds the socket taken, connects and [`send`]s
//! its request instead of starting a second app.
//!
//! A client keeps the connection open, without half-closing its write
//! side, until the reply arrives. Closing it means "stopped waiting" and
//! gets no reply.

use std::fmt;
use std::fs::{DirBuilder, Permissions};
use std::io::{self, BufRead as _, BufReader, ErrorKind, Read as _, Write as _};
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::{
    DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _,
};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures::channel::{mpsc, oneshot};
use futures::future::{self, Either};
use serde::{Deserialize, Serialize};

/// The version of the protocol this build speaks. A request of a newer
/// version is answered with an error asking to quit and reopen the app,
/// which was updated since it started.
pub const PROTOCOL_VERSION: u32 = 1;

/// How long a client waits to connect to a socket that is bound.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
/// How long the app waits for a connected client's request line.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// The longest request line the app reads.
const MAX_REQUEST_BYTES: u64 = 1 << 20;
/// How long [`claim_and_send`] waits between claims while the app quits.
const QUIT_RETRY_EVERY: Duration = Duration::from_millis(100);
/// How long it keeps claiming before it gives the quitting reply back.
const QUIT_WAIT: Duration = Duration::from_secs(5);
/// `sun_path` holds 104 bytes on macOS (108 on Linux); longer paths are
/// replaced by one in the temporary directory.
const MAX_SOCKET_PATH: usize = 100;
/// Only this user may connect to the socket, nor list its folder when
/// `luc` makes it.
const SOCKET_MODE: u32 = 0o600;
const DIR_MODE: u32 = 0o700;

/// What `luc` asks the running app to open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliRequest {
    pub version: u32,
    /// The directory `luc` ran in; relative `paths` are under it.
    pub cwd: PathBuf,
    /// Empty: `cwd` itself.
    pub paths: Vec<PathBuf>,
    /// Kept for `luc -n`; always `false` for now.
    #[serde(default)]
    pub new_window: bool,
    /// With no `paths`: bring the app forward as a click on its Dock icon
    /// does (spec M9) instead of opening `cwd`. Sent by `luc` in `/` with
    /// no path. The protocol stays at version 1: an app without the field
    /// ignores it.
    #[serde(default)]
    pub activate: bool,
}

impl CliRequest {
    /// A request of this build's [`PROTOCOL_VERSION`].
    pub fn new(cwd: PathBuf, paths: Vec<PathBuf>) -> CliRequest {
        CliRequest {
            version: PROTOCOL_VERSION,
            cwd,
            paths,
            new_window: false,
            activate: false,
        }
    }

    /// Brings the running app forward ([`CliRequest::activate`]).
    pub fn activate(cwd: PathBuf) -> CliRequest {
        CliRequest {
            activate: true,
            ..CliRequest::new(cwd, Vec::new())
        }
    }

    /// The paths to open: `paths` under `cwd`, or `cwd` when there are
    /// none.
    pub fn paths_to_open(&self) -> Vec<PathBuf> {
        if self.paths.is_empty() {
            vec![self.cwd.clone()]
        } else {
            self.paths.iter().map(|p| self.cwd.join(p)).collect()
        }
    }
}

/// Why an app refused a request, for a `luc` that can act on it. Added
/// after protocol version 1 without raising it: an older `luc` ignores the
/// field and prints `error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyStatus {
    /// The app is quitting and will not open anything (M1-2).
    Quitting,
}

/// The app's answer once it opened (or failed to open) the paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliReply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ReplyStatus>,
}

impl CliReply {
    pub fn ok() -> CliReply {
        CliReply {
            ok: true,
            error: None,
            status: None,
        }
    }

    pub fn error(message: impl Into<String>) -> CliReply {
        CliReply {
            ok: false,
            error: Some(message.into()),
            status: None,
        }
    }

    /// The answer of an app that is quitting. Its `error` is the text an
    /// older `luc` prints.
    pub fn quitting() -> CliReply {
        CliReply {
            status: Some(ReplyStatus::Quitting),
            ..CliReply::error("Lucerna is quitting")
        }
    }

    /// Whether the app said it is quitting.
    pub fn is_quitting(&self) -> bool {
        self.status == Some(ReplyStatus::Quitting)
    }
}

impl From<Result<(), String>> for CliReply {
    fn from(result: Result<(), String>) -> CliReply {
        match result {
            Ok(()) => CliReply::ok(),
            Err(err) => CliReply::error(err),
        }
    }
}

/// The answer to a request of `version` that this build cannot serve:
/// one from a newer `luc`. `None` when it can.
pub fn reply_for_version(version: u32) -> Option<CliReply> {
    (version > PROTOCOL_VERSION)
        .then(|| CliReply::error("Lucerna was updated. Quit Lucerna and open it again."))
}

/// The socket in `data_dir`: `luc.sock`, `luc-dev.sock` for a debug build,
/// so a dev build never hands paths to the release the user runs. The name
/// has no version: every version of a build channel shares one socket, so
/// updating Lucerna while the old app runs never makes a second app, and
/// the protocol `version` tells them apart. A path too long for `sun_path`
/// is replaced by one in a folder only this user can use
/// ([`fallback_dir`]), named by a hash of the full path.
pub fn socket_path(data_dir: &Path) -> PathBuf {
    let dev = if cfg!(debug_assertions) { "-dev" } else { "" };
    let name = format!("luc{dev}.sock");
    let path = data_dir.join(name);
    if path.as_os_str().len() <= MAX_SOCKET_PATH {
        return path;
    }
    fallback_dir().join(format!(
        "lucerna-{:016x}.sock",
        fnv1a(path.as_os_str().as_encoded_bytes())
    ))
}

/// Where a socket whose path is too long goes. macOS: `$TMPDIR`, a
/// folder of this user's own (`/var/folders/…/T`). Elsewhere `/tmp` is
/// shared: `$XDG_RUNTIME_DIR`, else `lucerna-<uid>` in the temporary
/// directory, which [`claim`] makes with mode 0700 and refuses when
/// another user owns it.
pub fn fallback_dir() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        std::env::temp_dir()
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join(format!("lucerna-{}", euid())))
    }
}

/// This process's effective user id.
fn euid() -> u32 {
    // SAFETY: `geteuid` cannot fail and touches no memory.
    unsafe { libc::geteuid() }
}

/// FNV-1a, 64 bits: a hash that stays the same across builds.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Who a [`claim`] made this process.
#[derive(Debug)]
pub enum Claim {
    /// The socket is bound: this process is the app.
    Server(Bound),
    /// An app listens on it: hand it the request.
    Client(UnixStream),
    /// No socket can be used (the data directory cannot be made, no
    /// permission): run the app without one.
    Unavailable(io::Error),
}

/// A socket this process bound: the listener, and the file it made.
#[derive(Debug)]
pub struct Bound {
    pub listener: UnixListener,
    pub file: SocketFile,
}

/// The file a [`Bound`] socket made: its path, and which file it was
/// (device and inode) when bound.
#[derive(Debug, Clone)]
pub struct SocketFile {
    path: PathBuf,
    id: (u64, u64),
}

impl SocketFile {
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Removes the file if it is still the one this process bound, under
    /// the lock [`claim`] takes; `false` when another app's socket (or
    /// nothing) is at the path now. A socket file of this app that
    /// another `luc` took for a crash's leftover was replaced: removing
    /// the path would cut the new app off from `luc`.
    ///
    /// # Errors
    /// When the lock cannot be taken or the file cannot be read or removed.
    pub fn remove(&self) -> io::Result<bool> {
        let _lock = lock(&self.path)?;
        match std::fs::symlink_metadata(&self.path) {
            Ok(meta) if file_id(&meta) == self.id => {
                std::fs::remove_file(&self.path)?;
                Ok(true)
            }
            Ok(_) => Ok(false),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err),
        }
    }
}

fn file_id(meta: &std::fs::Metadata) -> (u64, u64) {
    (meta.dev(), meta.ino())
}

/// Claims the socket at `path`, making its folder first (mode 0700; one
/// another user owns is refused): binds it (mode 0600), or connects to
/// the app that holds it. A socket file nobody listens on (left by a
/// crash) is removed and bound once more, under an exclusive lock on
/// `<path>.lock`: of several `luc` that find it so at once, one binds
/// and the others connect to it.
pub fn claim(path: &Path) -> Claim {
    claim_with(path, || {})
}

/// [`claim`], running `before_lock` once the socket was found stale and
/// before the lock is taken.
fn claim_with(path: &Path, before_lock: impl FnOnce()) -> Claim {
    if let Some(parent) = path.parent()
        && let Err(err) = private_dir(parent)
    {
        return Claim::Unavailable(err);
    }
    if let Some(claim) = attempt(path) {
        return claim;
    }
    before_lock();
    let _lock = match lock(path) {
        Ok(lock) => lock,
        Err(err) => return Claim::Unavailable(err),
    };
    // Another claimer may have bound it while this one waited.
    if let Some(claim) = attempt(path) {
        return claim;
    }
    tracing::info!(path = %path.display(), "removing a stale socket");
    if let Err(err) = std::fs::remove_file(path)
        && err.kind() != ErrorKind::NotFound
    {
        return Claim::Unavailable(err);
    }
    attempt(path).unwrap_or_else(|| {
        Claim::Unavailable(io::Error::new(
            ErrorKind::AddrInUse,
            "the luc socket stays refused",
        ))
    })
}

/// Binds `path`, or connects to the app bound to it; `None` when a
/// socket file is there that nobody listens on (or it went away in
/// between).
fn attempt(path: &Path) -> Option<Claim> {
    match bind(path) {
        Ok(bound) => Some(Claim::Server(bound)),
        Err(err) if err.kind() == ErrorKind::AddrInUse => match connect(path, CONNECT_TIMEOUT) {
            Ok(stream) => Some(Claim::Client(stream)),
            Err(err)
                if matches!(
                    err.kind(),
                    ErrorKind::ConnectionRefused | ErrorKind::NotFound
                ) =>
            {
                None
            }
            Err(err) => Some(Claim::Unavailable(err)),
        },
        Err(err) => Some(Claim::Unavailable(err)),
    }
}

/// Takes an exclusive lock on `<path>.lock` (mode 0600, made if missing),
/// held until the file returned is dropped.
fn lock(path: &Path) -> io::Result<std::fs::File> {
    let mut name = path.as_os_str().to_owned();
    name.push(".lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(SOCKET_MODE)
        .open(PathBuf::from(name))?;
    file.lock()?;
    Ok(file)
}

/// Makes `dir` and its missing parents with mode 0700; an error when it
/// belongs to another user.
fn private_dir(dir: &Path) -> io::Result<()> {
    DirBuilder::new()
        .recursive(true)
        .mode(DIR_MODE)
        .create(dir)?;
    if std::fs::metadata(dir)?.uid() != euid() {
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            format!("{} belongs to another user", dir.display()),
        ));
    }
    Ok(())
}

/// Binds `path` and makes it readable and writable by this user alone.
fn bind(path: &Path) -> io::Result<Bound> {
    let listener = UnixListener::bind(path)?;
    let made = std::fs::set_permissions(path, Permissions::from_mode(SOCKET_MODE))
        .and_then(|()| std::fs::symlink_metadata(path));
    match made {
        Ok(meta) => Ok(Bound {
            listener,
            file: SocketFile {
                path: path.to_path_buf(),
                id: file_id(&meta),
            },
        }),
        Err(err) => {
            std::fs::remove_file(path).ok();
            Err(err)
        }
    }
}

/// Gives the socket up for another process to claim: the file goes
/// first (if still ours, [`SocketFile::remove`]), then the listener
/// closes. The other way round, a `luc` that connects in between finds
/// the socket refusing, takes it for a crash's leftover and binds its
/// own, which the removal would then unlink.
pub fn release(bound: Bound) {
    release_with(bound, || {});
}

/// [`release`], running `between` after the file is removed and before
/// the listener closes.
fn release_with(bound: Bound, between: impl FnOnce()) {
    let path = bound.file.path();
    match bound.file.remove() {
        Ok(true) => {}
        Ok(false) => tracing::info!(path = %path.display(), "the luc socket is no longer ours"),
        Err(err) => {
            tracing::warn!(%err, path = %path.display(), "could not release the luc socket");
        }
    }
    between();
    drop(bound.listener);
}

/// Connects to `path`, giving up after `timeout`. The standard library
/// has no timed connect for Unix sockets, so it runs on a thread.
fn connect(path: &Path, timeout: Duration) -> io::Result<UnixStream> {
    let (tx, rx) = std::sync::mpsc::channel();
    let path = path.to_path_buf();
    std::thread::spawn(move || tx.send(UnixStream::connect(path)).ok());
    rx.recv_timeout(timeout)
        .unwrap_or_else(|_| Err(io::Error::new(ErrorKind::TimedOut, "connect timed out")))
}

/// Why a request got no reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
    /// No app could be reached.
    Unreachable(String),
    /// The request was delivered, but the app did not answer in time: it
    /// opens the paths once it is ready.
    NoReplyInTime,
    /// The connection closed or failed before any reply came: the
    /// listener that took it went away. Most often another `luc` held the
    /// socket for a moment and released it to the app it starts (M1-1);
    /// [`claim_and_send`] claims once more.
    Dropped(String),
    /// Anything else: a reply that does not parse.
    Failed(String),
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::Unreachable(err) | SendError::Dropped(err) | SendError::Failed(err) => {
                f.write_str(err)
            }
            SendError::NoReplyInTime => f.write_str("no reply from Lucerna in time"),
        }
    }
}

/// Sends `request` over `stream` and waits up to `timeout` for the reply.
///
/// # Errors
/// [`SendError::NoReplyInTime`] when the reply does not come within
/// `timeout`; [`SendError::Dropped`] when the connection closes or fails
/// before any reply; [`SendError::Failed`] for a reply that does not
/// parse.
pub fn send(
    mut stream: UnixStream,
    request: &CliRequest,
    timeout: Duration,
) -> Result<CliReply, SendError> {
    let failed = |what: &str, err: &dyn fmt::Display| SendError::Failed(format!("{what}: {err}"));
    // A connection the listener closed on is reset: on macOS even setting
    // its timeouts fails (EINVAL).
    let dropped = |what: &str, err: &dyn fmt::Display| SendError::Dropped(format!("{what}: {err}"));
    stream
        .set_write_timeout(Some(timeout))
        .and_then(|()| stream.set_read_timeout(Some(timeout)))
        .map_err(|err| dropped("socket", &err))?;
    let mut line = serde_json::to_string(request).map_err(|err| failed("request", &err))?;
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .map_err(|err| dropped("could not send the request", &err))?;
    let mut reply = String::new();
    match BufReader::new(stream).read_line(&mut reply) {
        Ok(_) => {}
        Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
            return Err(SendError::NoReplyInTime);
        }
        Err(err) => return Err(dropped("no reply from Lucerna", &err)),
    }
    if reply.is_empty() {
        return Err(SendError::Dropped("no reply from Lucerna".to_owned()));
    }
    serde_json::from_str(&reply).map_err(|err| failed("bad reply from Lucerna", &err))
}

/// How a [`claim_and_send`] ended.
#[derive(Debug)]
pub enum Handed {
    /// An app held the socket: its reply, or why none came.
    Reply(Result<CliReply, SendError>),
    /// Nobody did: this process bound it and is the app.
    Server(Bound),
    /// No socket can be used ([`Claim::Unavailable`]).
    Unavailable(io::Error),
}

/// [`claim`]s `path` and, when an app holds it, [`send`]s it `request`.
/// A connection dropped before any reply ([`SendError::Dropped`]) is
/// claimed once more (M1-1): the listener was most likely another `luc`'s,
/// held for a moment before it let the app it starts claim. The second
/// claim reaches that app, or binds when none came up.
///
/// Sending twice does not open a path twice. The app replies to every
/// request it reads, so a dropped connection means the request was never
/// read (the listener closed with it in its backlog) or the app died
/// before replying. And a path that is open already is only focused
/// (§6.3, rule 1).
///
/// An app that answers [`CliReply::quitting`] is still bound to the
/// socket for a moment. It is claimed again every [`QUIT_RETRY_EVERY`]
/// for up to [`QUIT_WAIT`] (M1-2; the limit bounds when the last retry
/// starts, not the total time, as each send may wait up to `timeout`),
/// until the app is gone and this process binds, or a new app answers;
/// after that the reply is returned as is.
pub fn claim_and_send(path: &Path, request: &CliRequest, timeout: Duration) -> Handed {
    claim_and_send_with(path, request, timeout, QUIT_WAIT)
}

fn claim_and_send_with(
    path: &Path,
    request: &CliRequest,
    timeout: Duration,
    quit_wait: Duration,
) -> Handed {
    let started = Instant::now();
    let mut again = true;
    loop {
        match claim(path) {
            Claim::Client(stream) => match send(stream, request, timeout) {
                Ok(reply) if reply.is_quitting() && started.elapsed() < quit_wait => {
                    tracing::info!(
                        path = %path.display(),
                        "the app is quitting; claiming the luc socket again"
                    );
                    std::thread::sleep(QUIT_RETRY_EVERY);
                }
                Err(SendError::Dropped(err)) if again => {
                    tracing::info!(
                        err,
                        path = %path.display(),
                        "the luc socket dropped the request; claiming it again"
                    );
                    again = false;
                }
                reply => return Handed::Reply(reply),
            },
            Claim::Server(bound) => return Handed::Server(bound),
            Claim::Unavailable(err) => return Handed::Unavailable(err),
        }
    }
}

/// A request and where its reply goes.
pub type CliMessage = (CliRequest, oneshot::Sender<CliReply>);

/// Accepts connections on `listener` on a thread of its own, and each on
/// another: reads the request line, sends it to `tx` (the router on the
/// main thread) and writes back the reply it gets. Never returns an error;
/// runs until the process exits.
pub fn serve(listener: UnixListener, tx: mpsc::UnboundedSender<CliMessage>) {
    let spawned = std::thread::Builder::new()
        .name("luc-socket".into())
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let tx = tx.clone();
                        std::thread::spawn(move || answer(stream, &tx));
                    }
                    Err(err) => tracing::warn!(%err, "luc socket: accept failed"),
                }
            }
        });
    if let Err(err) = spawned {
        tracing::error!(%err, "luc socket: could not start its thread");
    }
}

/// Answers one connection: one request line, one reply line. A process
/// of another user is refused before anything is read. A peer that closes
/// the connection before the reply is ready has stopped waiting and gets
/// no reply (M1-3).
fn answer(stream: UnixStream, tx: &mpsc::UnboundedSender<CliMessage>) {
    let allowed = peer_allowed(peer_uid(&stream), euid());
    let reply = match allowed.and_then(|()| read_request(&stream)) {
        Ok(request) => {
            // A dropped reply sender also covers the router discarding the
            // request for another reason than quitting. Answering
            // `quitting` is safe then: `luc` sends again, and opening a
            // path twice only focuses it (§6.3, rule 1).
            let (reply_tx, reply_rx) = oneshot::channel();
            if tx.unbounded_send((request, reply_tx)).is_err() {
                CliReply::quitting()
            } else {
                let gone = hangup(&stream);
                match futures::executor::block_on(future::select(reply_rx, gone)) {
                    Either::Left((reply, _)) => reply.unwrap_or_else(|_| CliReply::quitting()),
                    // Dropping the reply receiver tells the router that
                    // `luc` stopped waiting; the open goes on, and a
                    // failure is shown in the app (M1-3).
                    Either::Right((Ok(()), _)) => {
                        tracing::info!("luc stopped waiting; the open goes on");
                        return;
                    }
                    // No watcher (see `hangup`): wait for the reply alone.
                    Either::Right((Err(_), reply_rx)) => futures::executor::block_on(reply_rx)
                        .unwrap_or_else(|_| CliReply::quitting()),
                }
            }
        }
        Err(err) => CliReply::error(err),
    };
    let mut stream = stream;
    let line = serde_json::to_string(&reply).unwrap_or_else(|_| r#"{"ok":false}"#.to_owned());
    if let Err(err) = stream.write_all(format!("{line}\n").as_bytes()) {
        tracing::warn!(%err, "luc socket: could not reply");
    }
    // Wakes the `hangup` thread, whose clone keeps the connection open.
    stream.shutdown(std::net::Shutdown::Both).ok();
}

/// Resolves when the peer of `stream` has closed its end (M1-3). Starts
/// after the request line was read, so the byte it reads is never one the
/// request needs: `luc` sends nothing more. The thread ends with the
/// connection, which closes when [`answer`] returns or the peer hangs up.
/// Without a watcher it is canceled at once, and [`answer`] then waits
/// for the reply alone (as before).
fn hangup(stream: &UnixStream) -> oneshot::Receiver<()> {
    let (tx, rx) = oneshot::channel();
    // The timeout belongs to the socket, so it applies to every clone too.
    if stream.set_read_timeout(None).is_err() {
        return rx;
    }
    let Ok(mut watched) = stream.try_clone() else {
        return rx;
    };
    let spawned = std::thread::Builder::new()
        .name("luc-hangup".into())
        .spawn(move || {
            let mut byte = [0u8; 1];
            loop {
                match watched.read(&mut byte) {
                    Ok(0) => break,
                    Err(err)
                        if matches!(
                            err.kind(),
                            ErrorKind::Interrupted | ErrorKind::WouldBlock | ErrorKind::TimedOut
                        ) => {}
                    // Bytes `luc` should not send are skipped; any other
                    // error means the connection is gone.
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            tx.send(()).ok();
        });
    if spawned.is_err() {
        tracing::warn!("luc socket: could not watch for a hangup");
    }
    rx
}

/// The user id of the process at the other end of `stream`.
///
/// # Errors
/// When the platform cannot tell.
pub fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    let fd = stream.as_raw_fd();
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        let mut cred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: `cred` and `len` are valid for writes of the sizes given.
        let result = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&raw mut cred).cast(),
                &raw mut len,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(cred.uid)
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let (mut uid, mut gid) = (0, 0);
        // SAFETY: `uid` and `gid` are valid for writes.
        if unsafe { libc::getpeereid(fd, &raw mut uid, &raw mut gid) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(uid)
    }
}

/// Whether a peer of user `peer` may ask an app of user `ours` to open
/// paths: the same user only.
fn peer_allowed(peer: io::Result<u32>, ours: u32) -> Result<(), String> {
    match peer {
        Ok(uid) if uid == ours => Ok(()),
        Ok(uid) => {
            tracing::warn!(uid, "luc socket: refused a request of another user");
            Err("permission denied: another user's Lucerna".to_owned())
        }
        Err(err) => {
            tracing::warn!(%err, "luc socket: could not tell who connected");
            Err(format!("permission denied: {err}"))
        }
    }
}

fn read_request(stream: &UnixStream) -> Result<CliRequest, String> {
    stream
        .set_read_timeout(Some(READ_TIMEOUT))
        .map_err(|err| format!("socket: {err}"))?;
    let mut line = String::new();
    BufReader::new(stream.take(MAX_REQUEST_BYTES))
        .read_line(&mut line)
        .map_err(|err| format!("could not read the request: {err}"))?;
    serde_json::from_str(&line).map_err(|err| format!("bad request: {err}"))
}

#[cfg(test)]
mod tests {
    use futures::StreamExt as _;

    use std::sync::{Arc, Barrier};

    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    fn temp_socket() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sock");
        (dir, path)
    }

    #[test]
    fn claim_first_is_server() {
        let (_dir, path) = temp_socket();
        assert!(matches!(claim(&path), Claim::Server(_)));
        assert!(path.exists());
    }

    #[test]
    fn claim_makes_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a").join("b").join("t.sock");
        assert!(matches!(claim(&path), Claim::Server(_)));
    }

    #[test]
    fn claim_second_caller_becomes_client() {
        let (_dir, path) = temp_socket();
        let Claim::Server(_listener) = claim(&path) else {
            panic!("the first claim binds");
        };
        assert!(matches!(claim(&path), Claim::Client(_)));
    }

    #[test]
    fn claim_stale_socket_rebinds() {
        let (_dir, path) = temp_socket();
        // Bound, then dropped without removing the file: as after a crash.
        drop(UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        assert!(matches!(claim(&path), Claim::Server(_)));
    }

    #[test]
    fn request_round_trip() {
        let (_dir, path) = temp_socket();
        let Claim::Server(Bound { listener, .. }) = claim(&path) else {
            panic!("binds");
        };
        let (tx, mut rx) = mpsc::unbounded::<CliMessage>();
        serve(listener, tx);
        let server = std::thread::spawn(move || {
            let (request, reply) = futures::executor::block_on(rx.next()).unwrap();
            reply.send(CliReply::error("path not found: /x")).unwrap();
            request
        });
        let Claim::Client(stream) = claim(&path) else {
            panic!("the app listens");
        };
        let request = CliRequest::new(PathBuf::from("/w"), vec![PathBuf::from("/x")]);
        let reply = send(stream, &request, Duration::from_secs(5)).unwrap();
        assert_eq!(reply, CliReply::error("path not found: /x"));
        assert_eq!(server.join().unwrap(), request);
    }

    #[test]
    fn bad_request_gets_an_error_reply() {
        let (_dir, path) = temp_socket();
        let Claim::Server(Bound { listener, .. }) = claim(&path) else {
            panic!("binds");
        };
        let (tx, _rx) = mpsc::unbounded::<CliMessage>();
        serve(listener, tx);
        let mut stream = UnixStream::connect(&path).unwrap();
        stream.write_all(b"not json\n").unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let reply: CliReply = serde_json::from_str(&line).unwrap();
        assert!(!reply.ok);
        assert!(reply.error.unwrap().starts_with("bad request"));
    }

    #[test]
    fn unknown_fields_ignored() {
        let request: CliRequest = serde_json::from_str(
            r#"{"version":1,"cwd":"/w","paths":["a"],"env":{"X":"1"},"later":true}"#,
        )
        .unwrap();
        assert_eq!(
            request,
            CliRequest::new(PathBuf::from("/w"), vec![PathBuf::from("a")])
        );
        let reply: CliReply = serde_json::from_str(r#"{"ok":true,"window":3}"#).unwrap();
        assert_eq!(reply, CliReply::ok());
        assert_eq!(
            serde_json::to_string(&CliReply::ok()).unwrap(),
            r#"{"ok":true}"#
        );
    }

    #[test]
    fn paths_to_open_default_to_cwd() {
        let request = CliRequest::new(PathBuf::from("/w"), Vec::new());
        assert_eq!(request.paths_to_open(), vec![PathBuf::from("/w")]);
        let request = CliRequest::new(
            PathBuf::from("/w"),
            vec![PathBuf::from("a"), PathBuf::from("/b")],
        );
        assert_eq!(
            request.paths_to_open(),
            vec![PathBuf::from("/w/a"), PathBuf::from("/b")]
        );
    }

    #[test]
    fn newer_version_rejected() {
        assert_eq!(reply_for_version(PROTOCOL_VERSION), None);
        assert_eq!(reply_for_version(0), None);
        assert_eq!(
            reply_for_version(PROTOCOL_VERSION + 1),
            Some(CliReply::error(
                "Lucerna was updated. Quit Lucerna and open it again."
            ))
        );
    }

    #[test]
    fn socket_path_has_no_version() {
        let path = socket_path(Path::new("/d"));
        let expected = if cfg!(debug_assertions) {
            "luc-dev.sock"
        } else {
            "luc.sock"
        };
        assert_eq!(path, Path::new("/d").join(expected));
    }

    #[test]
    fn long_data_dir_uses_temp() {
        let long = PathBuf::from("/").join("d".repeat(120));
        let path = socket_path(&long);
        assert_eq!(path.parent(), Some(fallback_dir().as_path()));
        #[cfg(target_os = "macos")]
        assert_eq!(fallback_dir(), std::env::temp_dir(), "per-user on macOS");
        assert!(path.as_os_str().len() < 104, "{}", path.display());
        assert_eq!(path, socket_path(&long), "the same folder, the same socket");
        assert_ne!(path, socket_path(&long.join("x")));
    }

    #[test]
    fn socket_and_folder_are_private() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("data");
        let path = folder.join("t.sock");
        let Claim::Server(_listener) = claim(&path) else {
            panic!("binds");
        };
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&folder), 0o700);
        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn stale_socket_rebound_private() {
        let (_dir, path) = temp_socket();
        drop(UnixListener::bind(&path).unwrap());
        std::fs::set_permissions(&path, Permissions::from_mode(0o777)).unwrap();
        let Claim::Server(_listener) = claim(&path) else {
            panic!("rebinds");
        };
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn peer_uid_is_ours() {
        let (a, _b) = UnixStream::pair().unwrap();
        assert_eq!(peer_uid(&a).unwrap(), euid());
    }

    #[test]
    fn other_users_refused() {
        assert_eq!(peer_allowed(Ok(501), 501), Ok(()));
        assert!(peer_allowed(Ok(502), 501).is_err());
        assert!(peer_allowed(Ok(0), 501).is_err(), "not even root");
        assert!(peer_allowed(Err(io::Error::other("?")), 501).is_err());
    }

    #[test]
    fn release_keeps_a_new_claimers_socket() {
        let (_dir, path) = temp_socket();
        let Claim::Server(first) = claim(&path) else {
            panic!("binds");
        };
        // Another `luc` (or the app this one starts) claims while the
        // first lets go.
        let mut second = None;
        release_with(first, || second = Some(claim(&path)));
        let Some(Claim::Server(Bound {
            listener: second, ..
        })) = second
        else {
            panic!("the other claimer binds");
        };
        second.set_nonblocking(true).unwrap();
        let _client = UnixStream::connect(&path).expect("its socket is still there");
        assert!(second.accept().is_ok(), "and it is the one listening");
        assert!(matches!(claim(&path), Claim::Client(_)), "one app only");
    }

    #[test]
    fn send_times_out_after_delivery() {
        let (_dir, path) = temp_socket();
        let Claim::Server(Bound { listener, .. }) = claim(&path) else {
            panic!("binds");
        };
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            // Never replies while the client waits.
            std::thread::sleep(Duration::from_millis(500));
            line
        });
        let stream = UnixStream::connect(&path).unwrap();
        let request = CliRequest::new(PathBuf::from("/w"), Vec::new());
        assert_eq!(
            send(stream, &request, Duration::from_millis(100)),
            Err(SendError::NoReplyInTime)
        );
        assert!(!server.join().unwrap().is_empty(), "the request arrived");
    }

    #[test]
    fn activate_request() {
        let request = CliRequest::activate(PathBuf::from("/"));
        assert!(request.activate && request.paths.is_empty());
        let old: CliRequest =
            serde_json::from_str(r#"{"version":1,"cwd":"/w","paths":[]}"#).unwrap();
        assert!(!old.activate, "defaults to false");
    }

    #[test]
    fn racing_claims_on_a_stale_socket_make_one_app() {
        for _ in 0..20 {
            let (_dir, path) = temp_socket();
            // Left by a crash: bound, then dropped without removing it.
            drop(UnixListener::bind(&path).unwrap());
            // Both find it refused before either removes it.
            let barrier = Arc::new(Barrier::new(2));
            let threads: Vec<_> = (0..2)
                .map(|_| {
                    let (path, barrier) = (path.clone(), barrier.clone());
                    std::thread::spawn(move || {
                        claim_with(&path, || {
                            barrier.wait();
                        })
                    })
                })
                .collect();
            let claims: Vec<Claim> = threads.into_iter().map(|t| t.join().unwrap()).collect();
            let servers = claims
                .iter()
                .filter(|c| matches!(c, Claim::Server(_)))
                .count();
            let clients = claims
                .iter()
                .filter(|c| matches!(c, Claim::Client(_)))
                .count();
            assert_eq!((servers, clients), (1, 1), "{claims:?}");
        }
    }

    #[test]
    fn removal_keeps_a_socket_not_ours() {
        let (dir, path) = temp_socket();
        let Claim::Server(first) = claim(&path) else {
            panic!("binds");
        };
        // Another app took the path: the first file moved away (its inode
        // stays in use) and a new socket bound in its place.
        std::fs::rename(&path, dir.path().join("old.sock")).unwrap();
        let _other = UnixListener::bind(&path).unwrap();
        assert!(!first.file.remove().unwrap(), "not ours");
        assert!(path.exists(), "the other app's socket stays");
        release(first);
        assert!(path.exists(), "release keeps it too");
    }

    /// Whether a connection comes into `listener`'s backlog within
    /// `ms` milliseconds.
    fn pending(listener: &UnixListener, ms: i32) -> bool {
        let mut fd = libc::pollfd {
            fd: listener.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `fd` is one valid `pollfd`.
        unsafe { libc::poll(&raw mut fd, 1, ms) == 1 }
    }

    /// M1-1: a `luc` that connects while another `luc` holds the socket
    /// for a moment, before it releases it to the app it starts, sits in
    /// the backlog of a listener that closes without accepting it.
    #[test]
    fn release_drops_a_waiting_client() {
        let (_dir, path) = temp_socket();
        let Claim::Server(first) = claim(&path) else {
            panic!("binds");
        };
        let Claim::Client(stream) = claim(&path) else {
            panic!("connects to the first");
        };
        release(first);
        let request = CliRequest::new(PathBuf::from("/w"), Vec::new());
        let result = send(stream, &request, Duration::from_secs(1));
        assert!(matches!(result, Err(SendError::Dropped(_))), "{result:?}");
    }

    #[test]
    fn claim_and_send_reaches_the_app_that_came_up() {
        let (_dir, path) = temp_socket();
        let Claim::Server(first) = claim(&path) else {
            panic!("binds");
        };
        let request = CliRequest::new(PathBuf::from("/w"), vec![PathBuf::from("b")]);
        let client = {
            let (path, request) = (path.clone(), request.clone());
            std::thread::spawn(move || claim_and_send(&path, &request, Duration::from_secs(5)))
        };
        assert!(pending(&first.listener, 5_000), "the client connected");
        // The first `luc` lets go, and the app it started claims before
        // the listener with the second `luc` in its backlog closes.
        let (tx, mut rx) = mpsc::unbounded::<CliMessage>();
        release_with(first, || {
            let Claim::Server(app) = claim(&path) else {
                panic!("the app binds");
            };
            serve(app.listener, tx);
        });
        let (got_tx, got) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            while let Some((request, reply)) = futures::executor::block_on(rx.next()) {
                reply.send(CliReply::ok()).unwrap();
                got_tx.send(request).unwrap();
            }
        });
        let Handed::Reply(reply) = client.join().unwrap() else {
            panic!("an app held the socket");
        };
        assert_eq!(reply, Ok(CliReply::ok()));
        assert_eq!(got.recv_timeout(Duration::from_secs(1)), Ok(request));
        assert!(
            got.recv_timeout(Duration::from_millis(100)).is_err(),
            "sent once"
        );
    }

    #[test]
    fn claim_and_send_binds_when_no_app_came_up() {
        let (_dir, path) = temp_socket();
        let Claim::Server(first) = claim(&path) else {
            panic!("binds");
        };
        let client = {
            let path = path.clone();
            let request = CliRequest::new(PathBuf::from("/w"), Vec::new());
            std::thread::spawn(move || claim_and_send(&path, &request, Duration::from_secs(5)))
        };
        assert!(pending(&first.listener, 5_000), "the client connected");
        release(first);
        assert!(matches!(client.join().unwrap(), Handed::Server(_)));
    }

    #[test]
    fn claim_and_send_claims_again_only_once() {
        let (_dir, path) = temp_socket();
        let Claim::Server(Bound { listener, .. }) = claim(&path) else {
            panic!("binds");
        };
        let client = {
            let path = path.clone();
            let request = CliRequest::new(PathBuf::from("/w"), Vec::new());
            std::thread::spawn(move || claim_and_send(&path, &request, Duration::from_secs(5)))
        };
        // An app that closes every connection without a reply.
        for try_ in 1..=2 {
            assert!(pending(&listener, 5_000), "try {try_} connected");
            drop(listener.accept().unwrap());
        }
        let Handed::Reply(reply) = client.join().unwrap() else {
            panic!("an app held the socket");
        };
        assert!(matches!(reply, Err(SendError::Dropped(_))), "{reply:?}");
        assert!(!pending(&listener, 100), "no third try");
    }

    #[test]
    fn quitting_reply_wire_format() {
        assert_eq!(
            serde_json::to_string(&CliReply::quitting()).unwrap(),
            r#"{"ok":false,"error":"Lucerna is quitting","status":"quitting"}"#
        );
        let plain: CliReply = serde_json::from_str(r#"{"ok":false,"error":"x"}"#).unwrap();
        assert_eq!(plain.status, None);
        assert!(!plain.is_quitting());
        assert!(CliReply::quitting().is_quitting());
        assert_eq!(
            serde_json::to_string(&CliReply::ok()).unwrap(),
            r#"{"ok":true}"#
        );
    }

    #[test]
    fn serve_replies_quitting_when_the_router_is_gone() {
        let (_dir, path) = temp_socket();
        let Claim::Server(Bound { listener, .. }) = claim(&path) else {
            panic!("binds");
        };
        let (tx, rx) = mpsc::unbounded::<CliMessage>();
        drop(rx);
        serve(listener, tx);
        let request = CliRequest::new(PathBuf::from("/w"), Vec::new());
        let stream = connect(&path, Duration::from_secs(5)).unwrap();
        assert_eq!(
            send(stream, &request, Duration::from_secs(5)),
            Ok(CliReply::quitting())
        );
    }

    /// Polls `cond` every 10 ms for up to 2 s.
    fn eventually(mut cond: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(2);
        while Instant::now() < end {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        cond()
    }

    fn serve_one() -> (
        tempfile::TempDir,
        PathBuf,
        mpsc::UnboundedReceiver<CliMessage>,
    ) {
        let (dir, path) = temp_socket();
        let Claim::Server(Bound { listener, .. }) = claim(&path) else {
            panic!("binds");
        };
        let (tx, rx) = mpsc::unbounded::<CliMessage>();
        serve(listener, tx);
        (dir, path, rx)
    }

    /// M1-3: the router learns that `luc` stopped waiting.
    #[test]
    fn answer_drops_the_reply_when_luc_hangs_up() {
        let (_dir, path, mut rx) = serve_one();
        let request = CliRequest::new(PathBuf::from("/w"), vec![PathBuf::from("/x")]);
        let mut stream = connect(&path, Duration::from_secs(5)).unwrap();
        let line = serde_json::to_string(&request).unwrap();
        stream.write_all(format!("{line}\n").as_bytes()).unwrap();
        let (got, reply) = futures::executor::block_on(rx.next()).unwrap();
        assert_eq!(got, request);
        assert!(!reply.is_canceled());
        drop(stream);
        assert!(eventually(|| reply.is_canceled()), "luc hung up");
    }

    #[test]
    fn answer_keeps_the_reply_while_luc_waits() {
        let (_dir, path, mut rx) = serve_one();
        let request = CliRequest::new(PathBuf::from("/w"), vec![PathBuf::from("/x")]);
        let stream = connect(&path, Duration::from_secs(5)).unwrap();
        let client = std::thread::spawn(move || send(stream, &request, Duration::from_secs(10)));
        let (_, reply) = futures::executor::block_on(rx.next()).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(!reply.is_canceled());
        reply.send(CliReply::ok()).unwrap();
        assert_eq!(client.join().unwrap(), Ok(CliReply::ok()));
    }

    /// Reads one request line from the next connection and answers with
    /// `reply`.
    fn answer_one(listener: &UnixListener, reply: &CliReply, accepted: &AtomicUsize) {
        let (stream, _) = listener.accept().unwrap();
        accepted.fetch_add(1, Ordering::SeqCst);
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).unwrap();
        let mut stream = stream;
        let json = serde_json::to_string(reply).unwrap();
        stream.write_all(format!("{json}\n").as_bytes()).unwrap();
    }

    /// M1-2: the app that is quitting still holds the socket; `luc` claims
    /// it again until the app is gone and it can bind.
    #[test]
    fn claim_and_send_claims_past_a_quitting_app() {
        let (_dir, path) = temp_socket();
        let Claim::Server(app) = claim(&path) else {
            panic!("binds");
        };
        let request = CliRequest::new(PathBuf::from("/w"), Vec::new());
        let client = {
            let (path, request) = (path.clone(), request.clone());
            std::thread::spawn(move || {
                claim_and_send_with(
                    &path,
                    &request,
                    Duration::from_secs(5),
                    Duration::from_secs(5),
                )
            })
        };
        assert!(pending(&app.listener, 5_000), "the client connected");
        answer_one(&app.listener, &CliReply::quitting(), &AtomicUsize::new(0));
        release(app);
        assert!(matches!(client.join().unwrap(), Handed::Server(_)));
    }

    #[test]
    fn claim_and_send_gives_up_on_a_quitting_app() {
        let (_dir, path) = temp_socket();
        let Claim::Server(Bound { listener, .. }) = claim(&path) else {
            panic!("binds");
        };
        let connections = std::sync::Arc::new(AtomicUsize::new(0));
        std::thread::spawn({
            let connections = connections.clone();
            move || {
                loop {
                    answer_one(&listener, &CliReply::quitting(), &connections);
                }
            }
        });
        let request = CliRequest::new(PathBuf::from("/w"), Vec::new());
        let started = std::time::Instant::now();
        let handed = claim_and_send_with(
            &path,
            &request,
            Duration::from_secs(5),
            Duration::from_millis(300),
        );
        let Handed::Reply(reply) = handed else {
            panic!("an app held the socket");
        };
        assert_eq!(reply, Ok(CliReply::quitting()));
        assert!(started.elapsed() >= Duration::from_millis(300));
        assert!(connections.load(Ordering::SeqCst) >= 2, "claimed again");
    }

    #[test]
    fn removal_removes_our_socket() {
        let (_dir, path) = temp_socket();
        let Claim::Server(bound) = claim(&path) else {
            panic!("binds");
        };
        assert!(bound.file.remove().unwrap());
        assert!(!path.exists());
        assert!(!bound.file.remove().unwrap(), "already gone");
    }
}

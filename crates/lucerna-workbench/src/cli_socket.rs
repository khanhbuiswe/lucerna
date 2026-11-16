//! The socket `luc` hands its paths to the running app through (spec M13,
//! §6.5 of the multi-window design): one line of JSON each way.
//!
//! `luc` [`claim`]s the socket before anything else. Whoever binds it is
//! the app and passes the listener to `app::run`, which [`serve`]s it on a
//! thread; a later `luc` finds the socket taken, connects and [`send`]s
//! its request instead of starting a second app.

use std::fmt;
use std::fs::{DirBuilder, Permissions};
use std::io::{self, BufRead as _, BufReader, ErrorKind, Read as _, Write as _};
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _, PermissionsExt as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::channel::{mpsc, oneshot};
use serde::{Deserialize, Serialize};

/// The version of the protocol this build speaks. A request of a newer
/// version is answered with an error asking to update the app.
pub const PROTOCOL_VERSION: u32 = 1;

/// How long a client waits to connect to a socket that is bound.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
/// How long the app waits for a connected client's request line.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// The longest request line the app reads.
const MAX_REQUEST_BYTES: u64 = 1 << 20;
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
    /// ignores it, and the socket is per build anyway.
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

/// The app's answer once it opened (or failed to open) the paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliReply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl CliReply {
    pub fn ok() -> CliReply {
        CliReply {
            ok: true,
            error: None,
        }
    }

    pub fn error(message: impl Into<String>) -> CliReply {
        CliReply {
            ok: false,
            error: Some(message.into()),
        }
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
        .then(|| CliReply::error("this Lucerna is older than luc; update Lucerna"))
}

/// The socket in `data_dir`: `luc-<version>.sock`, `luc-<version>-dev.sock`
/// for a debug build, so a dev build never hands paths to the release the
/// user runs. A path too long for `sun_path` is replaced by one in a
/// folder only this user can use ([`fallback_dir`]), named by a hash of
/// the full path.
pub fn socket_path(data_dir: &Path) -> PathBuf {
    let dev = if cfg!(debug_assertions) { "-dev" } else { "" };
    let name = format!("luc-{}{dev}.sock", env!("CARGO_PKG_VERSION"));
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
    Server(UnixListener),
    /// An app listens on it: hand it the request.
    Client(UnixStream),
    /// No socket can be used (the data directory cannot be made, no
    /// permission): run the app without one.
    Unavailable(io::Error),
}

/// Claims the socket at `path`, making its folder first (mode 0700; one
/// another user owns is refused): binds it (mode 0600), or connects to
/// the app that holds it. A socket file nobody listens on (left by a
/// crash) is removed and bound once more.
pub fn claim(path: &Path) -> Claim {
    if let Some(parent) = path.parent()
        && let Err(err) = private_dir(parent)
    {
        return Claim::Unavailable(err);
    }
    match bind(path) {
        Ok(listener) => Claim::Server(listener),
        Err(err) if err.kind() == ErrorKind::AddrInUse => match connect(path, CONNECT_TIMEOUT) {
            Ok(stream) => Claim::Client(stream),
            Err(err) if err.kind() == ErrorKind::ConnectionRefused => {
                tracing::info!(path = %path.display(), "removing a stale socket");
                if let Err(err) = std::fs::remove_file(path) {
                    return Claim::Unavailable(err);
                }
                match bind(path) {
                    Ok(listener) => Claim::Server(listener),
                    Err(err) => Claim::Unavailable(err),
                }
            }
            Err(err) => Claim::Unavailable(err),
        },
        Err(err) => Claim::Unavailable(err),
    }
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
fn bind(path: &Path) -> io::Result<UnixListener> {
    let listener = UnixListener::bind(path)?;
    if let Err(err) = std::fs::set_permissions(path, Permissions::from_mode(SOCKET_MODE)) {
        std::fs::remove_file(path).ok();
        return Err(err);
    }
    Ok(listener)
}

/// Gives the socket up for another process to claim: the file goes
/// first, then the listener closes. The other way round, a `luc` that
/// connects in between finds the socket refusing, takes it for a crash's
/// leftover and binds its own, which the removal would then unlink.
pub fn release(listener: UnixListener, path: &Path) {
    release_with(listener, path, || {});
}

/// [`release`], running `between` after the file is removed and before
/// the listener closes.
fn release_with(listener: UnixListener, path: &Path, between: impl FnOnce()) {
    if let Err(err) = std::fs::remove_file(path) {
        tracing::warn!(%err, path = %path.display(), "could not release the luc socket");
    }
    between();
    drop(listener);
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
    /// Anything else: writing, reading, a reply that does not parse.
    Failed(String),
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SendError::Unreachable(err) | SendError::Failed(err) => f.write_str(err),
            SendError::NoReplyInTime => f.write_str("no reply from Lucerna in time"),
        }
    }
}

/// Sends `request` over `stream` and waits up to `timeout` for the reply.
///
/// # Errors
/// [`SendError::NoReplyInTime`] when the reply does not come within
/// `timeout`; [`SendError::Failed`] for the rest.
pub fn send(
    mut stream: UnixStream,
    request: &CliRequest,
    timeout: Duration,
) -> Result<CliReply, SendError> {
    let failed = |what: &str, err: &dyn fmt::Display| SendError::Failed(format!("{what}: {err}"));
    stream
        .set_write_timeout(Some(timeout))
        .and_then(|()| stream.set_read_timeout(Some(timeout)))
        .map_err(|err| failed("socket", &err))?;
    let mut line = serde_json::to_string(request).map_err(|err| failed("request", &err))?;
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .map_err(|err| failed("could not send the request", &err))?;
    let mut reply = String::new();
    match BufReader::new(stream).read_line(&mut reply) {
        Ok(_) => {}
        Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
            return Err(SendError::NoReplyInTime);
        }
        Err(err) => return Err(failed("no reply from Lucerna", &err)),
    }
    if reply.is_empty() {
        return Err(SendError::Failed("no reply from Lucerna".to_owned()));
    }
    serde_json::from_str(&reply).map_err(|err| failed("bad reply from Lucerna", &err))
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
/// of another user is refused before anything is read.
fn answer(stream: UnixStream, tx: &mpsc::UnboundedSender<CliMessage>) {
    let allowed = peer_allowed(peer_uid(&stream), euid());
    let reply = match allowed.and_then(|()| read_request(&stream)) {
        Ok(request) => {
            let (reply_tx, reply_rx) = oneshot::channel();
            if tx.unbounded_send((request, reply_tx)).is_err() {
                CliReply::error("Lucerna is quitting")
            } else {
                futures::executor::block_on(reply_rx)
                    .unwrap_or_else(|_| CliReply::error("Lucerna is quitting"))
            }
        }
        Err(err) => CliReply::error(err),
    };
    let mut stream = stream;
    let line = serde_json::to_string(&reply).unwrap_or_else(|_| r#"{"ok":false}"#.to_owned());
    if let Err(err) = stream.write_all(format!("{line}\n").as_bytes()) {
        tracing::warn!(%err, "luc socket: could not reply");
    }
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
        let Claim::Server(listener) = claim(&path) else {
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
        let Claim::Server(listener) = claim(&path) else {
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
                "this Lucerna is older than luc; update Lucerna"
            ))
        );
    }

    #[test]
    fn socket_path_is_versioned() {
        let path = socket_path(Path::new("/d"));
        let name = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(path.parent(), Some(Path::new("/d")));
        assert!(name.starts_with(&format!("luc-{}", env!("CARGO_PKG_VERSION"))));
        assert_eq!(name.ends_with("-dev.sock"), cfg!(debug_assertions));
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
        release_with(first, &path, || second = Some(claim(&path)));
        let Some(Claim::Server(second)) = second else {
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
        let Claim::Server(listener) = claim(&path) else {
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
}

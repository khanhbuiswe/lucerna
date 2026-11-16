//! The socket `luc` hands its paths to the running app through (spec M13,
//! §6.5 of the multi-window design): one line of JSON each way.
//!
//! `luc` [`claim`]s the socket before anything else. Whoever binds it is
//! the app and passes the listener to `app::run`, which [`serve`]s it on a
//! thread; a later `luc` finds the socket taken, connects and [`send`]s
//! its request instead of starting a second app.

use std::io::{self, BufRead as _, BufReader, ErrorKind, Read as _, Write as _};
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
}

impl CliRequest {
    /// A request of this build's [`PROTOCOL_VERSION`].
    pub fn new(cwd: PathBuf, paths: Vec<PathBuf>) -> CliRequest {
        CliRequest {
            version: PROTOCOL_VERSION,
            cwd,
            paths,
            new_window: false,
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
/// user runs. A path too long for `sun_path` is replaced by one in the
/// temporary directory, named by a hash of the full path.
pub fn socket_path(data_dir: &Path) -> PathBuf {
    let dev = if cfg!(debug_assertions) { "-dev" } else { "" };
    let name = format!("luc-{}{dev}.sock", env!("CARGO_PKG_VERSION"));
    let path = data_dir.join(name);
    if path.as_os_str().len() <= MAX_SOCKET_PATH {
        return path;
    }
    std::env::temp_dir().join(format!(
        "lucerna-{:016x}.sock",
        fnv1a(path.as_os_str().as_encoded_bytes())
    ))
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

/// Claims the socket at `path`, making its folder first: binds it, or
/// connects to the app that holds it. A socket file nobody listens on
/// (left by a crash) is removed and bound once more.
pub fn claim(path: &Path) -> Claim {
    if let Some(parent) = path.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        return Claim::Unavailable(err);
    }
    match UnixListener::bind(path) {
        Ok(listener) => Claim::Server(listener),
        Err(err) if err.kind() == ErrorKind::AddrInUse => match connect(path, CONNECT_TIMEOUT) {
            Ok(stream) => Claim::Client(stream),
            Err(err) if err.kind() == ErrorKind::ConnectionRefused => {
                tracing::info!(path = %path.display(), "removing a stale socket");
                if let Err(err) = std::fs::remove_file(path) {
                    return Claim::Unavailable(err);
                }
                match UnixListener::bind(path) {
                    Ok(listener) => Claim::Server(listener),
                    Err(err) => Claim::Unavailable(err),
                }
            }
            Err(err) => Claim::Unavailable(err),
        },
        Err(err) => Claim::Unavailable(err),
    }
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

/// Sends `request` over `stream` and waits up to `timeout` for the reply.
///
/// # Errors
/// What went wrong writing, reading or parsing.
pub fn send(
    mut stream: UnixStream,
    request: &CliRequest,
    timeout: Duration,
) -> Result<CliReply, String> {
    stream
        .set_write_timeout(Some(timeout))
        .and_then(|()| stream.set_read_timeout(Some(timeout)))
        .map_err(|err| format!("socket: {err}"))?;
    let mut line = serde_json::to_string(request).map_err(|err| err.to_string())?;
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .map_err(|err| format!("could not send the request: {err}"))?;
    let mut reply = String::new();
    BufReader::new(stream)
        .read_line(&mut reply)
        .map_err(|err| format!("no reply from Lucerna: {err}"))?;
    if reply.is_empty() {
        return Err("no reply from Lucerna".to_owned());
    }
    serde_json::from_str(&reply).map_err(|err| format!("bad reply from Lucerna: {err}"))
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

/// Answers one connection: one request line, one reply line.
fn answer(stream: UnixStream, tx: &mpsc::UnboundedSender<CliMessage>) {
    let reply = match read_request(&stream) {
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
        assert_eq!(path.parent(), Some(std::env::temp_dir().as_path()));
        assert!(path.as_os_str().len() < 104, "{}", path.display());
        assert_eq!(path, socket_path(&long), "the same folder, the same socket");
        assert_ne!(path, socket_path(&long.join("x")));
    }
}

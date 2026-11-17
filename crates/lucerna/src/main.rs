use std::path::PathBuf;
use std::time::Instant;

use clap::{Parser, Subcommand};
use lucerna::{Target, logging, prompts_for_folder, resolve_target};
use lucerna::{ext_cli, lsp_cli};
#[cfg(not(unix))]
use lucerna_workbench::app;
use lucerna_workbench::app::Launch;

/// Lucerna: a read-only code viewer.
#[derive(Debug, Parser)]
#[command(name = "luc", version, args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// File or directory to open. A folder literally named `ext` or `lsp`
    /// must be opened as `luc ./ext` or `luc ./lsp`.
    path: Option<PathBuf>,
    /// Print `startup_ms=<n>` to stderr once the first frame is painted.
    /// Runs the app in this process, like `--foreground`: it refuses when
    /// an app already runs.
    #[arg(long)]
    trace_startup: bool,
    /// Run the app in this process, tied to the terminal, instead of
    /// starting one apart from the terminal. For benchmarks and debugging.
    /// It refuses when an app already runs: it never hands the path over.
    #[arg(long, hide = true)]
    foreground: bool,
    /// Set by `luc` on the app process it starts apart from the terminal:
    /// hand the request to an app that came up meanwhile, else run the app
    /// here.
    #[arg(long, hide = true)]
    detached_child: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Search, install, update and remove extensions.
    Ext(ext_cli::ExtArgs),
    /// Language server tools: `luc lsp health` lists installed servers.
    Lsp(lsp_cli::LspArgs),
}

fn main() {
    let start = Instant::now();
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Ext(args)) => std::process::exit(ext_cli::run(args)),
        Some(Command::Lsp(args)) => std::process::exit(lsp_cli::run(args)),
        None => {}
    }
    let guard = logging::init();
    let code = run(cli, start);
    drop(guard);
    if code != 0 {
        std::process::exit(code);
    }
}

/// `luc [PATH]`: resolves the path, then hands it to the running app,
/// starts one apart from the terminal, or runs the app here. The exit
/// code.
fn run(cli: Cli, start: Instant) -> i32 {
    let on_first_frame = cli.trace_startup.then(|| -> Box<dyn FnOnce()> {
        Box::new(move || {
            let startup_ms = start.elapsed().as_millis();
            tracing::info!(startup_ms, "first frame painted");
            eprintln!("startup_ms={startup_ms}");
        })
    });
    let cwd = std::env::current_dir().ok();
    // Finder and the Dock start the app bundle in `/` with no path: there
    // is no terminal to leave, and nothing to open yet.
    let from_finder = cwd
        .as_deref()
        .is_some_and(|cwd| prompts_for_folder(cli.path.as_deref(), cwd));
    let launch = if from_finder {
        tracing::info!("starting without a path");
        None
    } else {
        let Target {
            root,
            file,
            single_file,
        } = match resolve_target(cli.path) {
            Ok(target) => target,
            Err(err) => {
                tracing::error!(err, "could not open");
                eprintln!("luc: {err}");
                return 1;
            }
        };
        tracing::info!(root = %root.display(), file = ?file, single_file, "starting");
        Some(Launch {
            root,
            file,
            single_file,
            on_first_frame,
        })
    };
    #[cfg(unix)]
    {
        let mode = if cli.trace_startup {
            socket::RunMode::Exclusive {
                refusal: socket::RUNNING_TRACE,
            }
        } else if cli.foreground {
            socket::RunMode::Exclusive {
                refusal: socket::RUNNING_FOREGROUND,
            }
        } else if cli.detached_child || from_finder {
            socket::RunMode::Here
        } else {
            socket::RunMode::Detach
        };
        socket::run(launch, cwd, mode)
    }
    #[cfg(not(unix))]
    {
        let _ = (cwd, cli.foreground, cli.detached_child);
        app::run(launch, None);
        0
    }
}

/// The socket `luc` and the running app share (spec M13, M18, §6.5).
#[cfg(unix)]
mod socket {
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use lucerna::launch::{send_with_retry, spawn_detached};
    use lucerna_workbench::app::{self, CliListener, Launch};
    use lucerna_workbench::cli_socket::{
        Claim, CliReply, CliRequest, Handed, SendError, claim, claim_and_send, release, socket_path,
    };
    use lucerna_workbench::default_data_dir;

    /// How long `luc` waits for the running app's reply. Past it the
    /// request is delivered all the same: the app opens the path once it
    /// is ready.
    const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
    /// How long `luc` tries to connect to an app it started (spec §6.5).
    const START_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
    /// Then how long it waits for that app's reply: it binds the socket
    /// before its first window, so the reply also waits for the start.
    const START_REPLY_TIMEOUT: Duration = Duration::from_secs(10);

    /// The flag the app `luc` starts apart from the terminal runs with.
    pub const DETACHED_CHILD: &str = "--detached-child";
    /// Said for `--trace-startup` when an app already holds the socket.
    pub const RUNNING_TRACE: &str = "Lucerna is already running; quit it to measure startup.";
    /// Said for `--foreground` when an app already holds the socket.
    pub const RUNNING_FOREGROUND: &str =
        "Lucerna is already running; quit it to run in the foreground.";

    /// What `luc` does once it holds the socket.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum RunMode {
        /// A terminal `luc`: start the app apart from the terminal and
        /// hand it the request.
        Detach,
        /// Run the app in this process: the app `luc` started, or a launch
        /// from Finder. An app that holds the socket gets the request.
        Here,
        /// `--foreground` and `--trace-startup`: run the app here, or
        /// refuse with `refusal` (exit 1) when an app holds the socket,
        /// sending nothing: the user means to measure or debug this
        /// process.
        Exclusive { refusal: &'static str },
    }

    /// Claims the socket, then: hands `launch` to the app that holds it
    /// (not in `Exclusive` mode, which refuses); runs the app here
    /// (`Here`, `Exclusive`, or no socket to use); or starts the app apart
    /// from the terminal and hands it `launch` (`Detach`).
    pub fn run(launch: Option<Launch>, cwd: Option<PathBuf>, mode: RunMode) -> i32 {
        let Some(data_dir) = default_data_dir() else {
            tracing::warn!("no HOME: running without the luc socket");
            app::run(launch, None);
            return 0;
        };
        let socket = socket_path(&data_dir);
        if let RunMode::Exclusive { refusal } = mode {
            return match claim(&socket) {
                Claim::Server(bound) => {
                    app::run(launch, Some(bound));
                    0
                }
                Claim::Client(_) => {
                    eprintln!("luc: {refusal}");
                    1
                }
                Claim::Unavailable(err) => {
                    tracing::warn!(%err, socket = %socket.display(), "running without the luc socket");
                    eprintln!("luc: warning: running without the luc socket: {err}");
                    app::run(launch, None);
                    0
                }
            };
        }
        let request = match &launch {
            Some(launch) => request_for(launch, cwd.clone()),
            // `luc` in `/` with no path: `/` is not opened; the app comes
            // forward as from its Dock icon.
            None => CliRequest::activate(cwd.clone().unwrap_or_else(|| PathBuf::from("/"))),
        };
        // An app that holds the socket gets the request; a connection
        // another `luc` dropped while handing the socket to the app it
        // starts is claimed once more (M1-1), and so is an app that answers
        // that it is quitting, every 100 ms for up to 5 s (M1-2).
        match claim_and_send(&socket, &request, REPLY_TIMEOUT) {
            Handed::Reply(reply) => {
                tracing::info!(socket = %socket.display(), "handed the request to the running app");
                reply_code(reply)
            }
            Handed::Server(listener) => match launch {
                Some(launch) if mode == RunMode::Detach => detach(listener, &socket, &launch, cwd),
                launch => {
                    app::run(launch, Some(listener));
                    0
                }
            },
            Handed::Unavailable(err) => {
                tracing::warn!(%err, socket = %socket.display(), "running without the luc socket");
                eprintln!("luc: warning: running without the luc socket: {err}");
                app::run(launch, None);
                0
            }
        }
    }

    /// Starts the app apart from the terminal (`luc --detached-child
    /// <path>`) and hands it `launch` once it listens.
    fn detach(listener: CliListener, socket: &Path, launch: &Launch, cwd: Option<PathBuf>) -> i32 {
        // For the app to claim: the file first, then the listener.
        release(listener);
        if let Err(err) = start_detached(launch, cwd.as_deref()) {
            tracing::error!(%err, "could not start the app");
            eprintln!("luc: could not start Lucerna: {err}");
            return 1;
        }
        let request = request_for(launch, cwd);
        match send_with_retry(socket, &request, START_CONNECT_TIMEOUT, START_REPLY_TIMEOUT) {
            Err(SendError::Unreachable(err)) => {
                tracing::error!(err, "the app did not listen");
                eprintln!("luc: Lucerna is starting; could not confirm the window opened");
                1
            }
            reply => reply_code(reply),
        }
    }

    /// Starts this binary apart from the terminal on `launch`, with
    /// [`DETACHED_CHILD`], in `cwd` (else `/`).
    fn start_detached(launch: &Launch, cwd: Option<&Path>) -> std::io::Result<()> {
        let exe = std::env::current_exe()?;
        let path = launch_path(launch);
        let dir = cwd.unwrap_or(Path::new("/"));
        let args = [DETACHED_CHILD.into(), path.clone().into_os_string()];
        spawn_detached(&exe, &args, dir)?;
        tracing::info!(path = %path.display(), "started the app apart from the terminal");
        Ok(())
    }

    /// The path a [`Launch`] is on: the file opened alone, else the
    /// folder.
    fn launch_path(launch: &Launch) -> PathBuf {
        match (&launch.file, launch.single_file) {
            (Some(file), true) => file.clone(),
            _ => launch.root.clone(),
        }
    }

    fn request_for(launch: &Launch, cwd: Option<PathBuf>) -> CliRequest {
        let path = launch_path(launch);
        CliRequest::new(cwd.unwrap_or_else(|| path.clone()), vec![path])
    }

    /// 0 when the app opened the paths, or has the request and opens them
    /// once it is ready; otherwise prints why and 1.
    fn reply_code(reply: Result<CliReply, SendError>) -> i32 {
        match reply {
            Ok(CliReply { ok: true, .. }) => 0,
            Ok(CliReply { error, .. }) => {
                let error = error.unwrap_or_else(|| "Lucerna could not open it".to_owned());
                for line in error.lines() {
                    eprintln!("luc: {line}");
                }
                1
            }
            Err(SendError::NoReplyInTime) => {
                tracing::warn!("the app did not reply in time");
                eprintln!("luc: Lucerna is busy; the path will open when it is ready");
                0
            }
            Err(err) => {
                eprintln!("luc: {err}");
                1
            }
        }
    }
}

#![cfg(unix)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender, unbounded};
use lucerna_lsp::process::spawn_process;
use lucerna_lsp::transport::{Inbox, LaunchSpec};

#[derive(Debug, PartialEq)]
enum Event {
    Message(String),
    Stderr(String),
    Closed(Option<i32>),
}

struct Recorder(Sender<Event>);

impl Inbox for Recorder {
    fn message(&self, body: String) {
        let _ = self.0.send(Event::Message(body));
    }
    fn stderr(&self, line: String) {
        let _ = self.0.send(Event::Stderr(line));
    }
    fn closed(&self, exit_code: Option<i32>) {
        let _ = self.0.send(Event::Closed(exit_code));
    }
}

fn spec(args: &[&str]) -> LaunchSpec {
    LaunchSpec {
        server: "test".into(),
        program: "/bin/sh".into(),
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        env: HashMap::from([("PATH".to_owned(), "/usr/bin:/bin".to_owned())]),
        cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
    }
}

fn recv(rx: &Receiver<Event>) -> Event {
    rx.recv_timeout(Duration::from_secs(5))
        .expect("event in time")
}

#[test]
fn echo_round_trip_then_stderr_and_exit_code() {
    let (tx, rx) = unbounded();
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/echo_lsp.sh");
    let transport = spawn_process(&spec(&[script]), Arc::new(Recorder(tx))).unwrap();
    let body = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
    transport.send(body.to_owned());
    assert_eq!(recv(&rx), Event::Message(body.to_owned()));
    assert_eq!(recv(&rx), Event::Stderr("boom".into()));
    assert_eq!(recv(&rx), Event::Closed(Some(3)));
    assert!(
        rx.recv_timeout(Duration::from_millis(200)).is_err(),
        "closed is last and once"
    );
}

#[test]
fn kill_closes() {
    let (tx, rx) = unbounded();
    let transport = spawn_process(&spec(&["-c", "sleep 30"]), Arc::new(Recorder(tx))).unwrap();
    transport.kill();
    let event = rx
        .recv_timeout(Duration::from_secs(2))
        .expect("closed within 2s");
    assert!(matches!(event, Event::Closed(_)), "{event:?}");
}

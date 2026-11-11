//! A server's events on the UI thread: diagnostics (batched), progress,
//! logs, watch registrations and exit.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Weak};
use std::task::Poll;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{Context, Task};
use lsp_types::MessageType;
use lucerna_lsp::diagnostics::Diagnostic;
use lucerna_lsp::position::uri_to_path;
use lucerna_lsp::{LanguageServer, LogKind, LogLine, ServerEvent, ServerKey, Slot};

use super::{LanguageServers, LanguageServersEvent};
use crate::workspace_state::absolute_path;

/// Lines kept in each server's log.
const LOG_LINES: usize = 2000;
/// The most characters of the progress text, before the `+N`.
const PROGRESS_TEXT_MAX: usize = 60;
/// Lines of stderr shown with a failed server.
const STDERR_TAIL: usize = 50;
const DIAGNOSTICS_BATCH: Duration = Duration::from_millis(50);

/// A server's event stream and the task pumping it. Closing the stream
/// before a shutdown keeps the shutdown thread's last events (exit, log)
/// from waking the UI executor from another thread.
pub(super) struct Pump {
    events: Rc<RefCell<Option<UnboundedReceiver<ServerEvent>>>>,
    _task: Task<()>,
}

impl Drop for Pump {
    fn drop(&mut self) {
        // Senders now fail instead of waking the cancelled task.
        self.events.borrow_mut().take();
    }
}

/// One `$/progress` token of a server.
pub(super) struct ProgressItem {
    /// When it last changed: bigger is newer, across servers.
    pub(super) updated: u64,
    token: String,
    title: Option<String>,
    message: Option<String>,
    percentage: Option<u32>,
}

impl ProgressItem {
    pub(super) fn text(&self) -> String {
        let mut text = self.title.clone().unwrap_or_default();
        if let Some(percentage) = self.percentage {
            if !text.is_empty() {
                text.push(' ');
            }
            text.push_str(&format!("{percentage}%"));
        }
        if let Some(message) = &self.message {
            if !text.is_empty() {
                text.push_str(" — ");
            }
            text.push_str(message);
        }
        text
    }
}

impl LanguageServers {
    /// The status bar's progress text: the newest unfinished `$/progress`
    /// of any server, `"{server}: {title} {percentage}% — {message}"`, or
    /// `"Starting {server}…"` while one starts; cut at 60 characters, and
    /// `" +N"` when N more are under way. `None` when nothing is.
    pub fn progress_text(&self) -> Option<String> {
        let starting = self
            .registry
            .keys()
            .filter(|key| matches!(self.registry.slot(key), Some(Slot::Starting)));
        summarize_progress(&self.progress, starting)
    }

    /// Delivers a server's events on the UI thread until it is gone. Holds
    /// the server weakly, so dropping it ends the stream.
    pub(super) fn pump(
        &mut self,
        key: ServerKey,
        generation: u64,
        server: Weak<LanguageServer>,
        events: UnboundedReceiver<ServerEvent>,
        cx: &mut Context<Self>,
    ) {
        let events = Rc::new(RefCell::new(Some(events)));
        let (stream, pump_key) = (events.clone(), key.clone());
        let task = cx.spawn(async move |this, cx| {
            let mut stream =
                futures::stream::poll_fn(move |task_cx| match stream.borrow_mut().as_mut() {
                    Some(events) => events.poll_next_unpin(task_cx),
                    None => Poll::Ready(None),
                });
            while let Some(event) = stream.next().await {
                let delivered = this.update(cx, |servers, cx| {
                    servers.server_event(&key, generation, &server, event, cx)
                });
                if delivered.is_err() {
                    break;
                }
            }
        });
        // Replacing an older pump drops it, and its stream with it.
        self.pumps.insert(
            pump_key,
            Pump {
                events,
                _task: task,
            },
        );
    }

    fn server_event(
        &mut self,
        key: &ServerKey,
        generation: u64,
        server: &Weak<LanguageServer>,
        event: ServerEvent,
        cx: &mut Context<Self>,
    ) {
        let current = self.is_current(key, generation);
        match event {
            ServerEvent::Log(line) => self.push_log(&key.server, line.kind, line.text, cx),
            ServerEvent::ShowMessage { kind, message } => {
                if current && kind == MessageType::ERROR {
                    self.errors
                        .entry(key.clone())
                        .or_default()
                        .push(message.clone());
                    cx.emit(LanguageServersEvent::StatusChanged);
                    cx.notify();
                }
                self.push_log(&key.server, LogKind::Server, message, cx);
            }
            _ if !current => {}
            ServerEvent::Exited(code) => {
                if matches!(
                    self.registry.slot(key),
                    Some(Slot::Running(_) | Slot::Starting)
                ) {
                    let message = match code {
                        Some(code) => format!("exited with code {code}"),
                        None => "exited".to_owned(),
                    };
                    self.set_failed(key, message, cx);
                }
            }
            ServerEvent::Diagnostics(params) => {
                let Some(server) = server.upgrade() else {
                    return;
                };
                let Some(file) = uri_to_path(&params.uri) else {
                    return;
                };
                let file = absolute_path(&self.root, &file);
                let open = self
                    .docs
                    .get(file.as_path())
                    .and_then(|doc| Some((&doc.snapshot, self.sync.version(&file, key)?)));
                if let Some(path) = self
                    .diagnostics
                    .publish(key, params, server.encoding(), open)
                {
                    self.diagnostics_changed([Arc::from(path)], cx);
                }
            }
            ServerEvent::Progress {
                token,
                title,
                message,
                percentage,
                done,
            } => {
                let items = self.progress.entry(key.clone()).or_default();
                let existing = items.iter().position(|item| item.token == token);
                let mut item = match existing {
                    Some(i) => items.remove(i),
                    None => ProgressItem {
                        updated: 0,
                        token,
                        title: None,
                        message: None,
                        percentage: None,
                    },
                };
                if !done {
                    self.progress_clock += 1;
                    item.updated = self.progress_clock;
                    item.title = title.or(item.title);
                    item.message = message.or(item.message);
                    item.percentage = percentage.or(item.percentage);
                    items.push(item);
                }
                cx.emit(LanguageServersEvent::StatusChanged);
                cx.notify();
            }
            ServerEvent::WatchedFilesRegistered { id, watchers } => {
                self.watched.register(key.clone(), id, watchers);
            }
            ServerEvent::WatchedFilesUnregistered { id } => {
                self.watched.unregister(key, &id);
            }
        }
    }

    pub(super) fn stderr_tail(&self, server: &str) -> Vec<String> {
        let Some(log) = self.logs.get(server) else {
            return Vec::new();
        };
        let mut tail: Vec<String> = log
            .iter()
            .rev()
            .filter(|line| line.kind == LogKind::Stderr)
            .take(STDERR_TAIL)
            .map(|line| line.text.clone())
            .collect();
        tail.reverse();
        tail
    }

    pub(super) fn push_log(
        &mut self,
        server: &Arc<str>,
        kind: LogKind,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let log = self.logs.entry(server.clone()).or_default();
        if log.len() == LOG_LINES {
            log.pop_front();
        }
        log.push_back(LogLine { kind, text });
        cx.emit(LanguageServersEvent::LogChanged(server.clone()));
    }

    /// Queues `paths` for the next [`LanguageServersEvent::DiagnosticsChanged`],
    /// sent 50 ms after the first of a batch.
    pub(super) fn diagnostics_changed(
        &mut self,
        paths: impl IntoIterator<Item = Arc<Path>>,
        cx: &mut Context<Self>,
    ) {
        let before = self.changed_diagnostics.len();
        self.changed_diagnostics.extend(paths);
        if self.changed_diagnostics.len() == before || self.diagnostics_flush_pending {
            return;
        }
        self.diagnostics_flush_pending = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DIAGNOSTICS_BATCH).await;
            this.update(cx, |servers, cx| {
                servers.diagnostics_flush_pending = false;
                servers.diagnostic_counts = servers.diagnostics.summary();
                let paths = std::mem::take(&mut servers.changed_diagnostics);
                for path in &paths {
                    servers.push_diagnostics(path, cx);
                }
                if !paths.is_empty() {
                    cx.emit(LanguageServersEvent::DiagnosticsChanged(
                        paths.into_iter().collect(),
                    ));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Gives every view of the open document `path` its diagnostics from
    /// all servers; switches their icon column on while it has servers,
    /// off when it has neither servers nor diagnostics. Unopened paths are
    /// skipped. Raw entries (published before the server had the file)
    /// are converted with the open text first, so `for_file` never reads
    /// the disk on the UI thread.
    pub(super) fn push_diagnostics(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get(path) else {
            return;
        };
        let has_servers = !doc.keys.is_empty();
        let snapshot = doc.snapshot.clone();
        self.diagnostics.resolve_open(path, &snapshot);
        let diagnostics: Arc<[Diagnostic]> = self.diagnostics.for_file(path).into();
        let source = has_servers || !diagnostics.is_empty();
        for view in self.documents.read(cx).views_of(path) {
            let diagnostics = diagnostics.clone();
            view.update(cx, |view, cx| {
                if source {
                    view.set_diagnostics(diagnostics, cx);
                } else {
                    view.clear_diagnostics(cx);
                }
            });
        }
    }
}

/// See [`LanguageServers::progress_text`]. A starting server that already
/// reports progress is counted once, by its progress.
fn summarize_progress<'a>(
    progress: &'a HashMap<ServerKey, Vec<ProgressItem>>,
    starting: impl Iterator<Item = &'a ServerKey>,
) -> Option<String> {
    let mut items: Vec<(&str, &ProgressItem)> = progress
        .iter()
        .flat_map(|(key, items)| items.iter().map(|item| (&*key.server, item)))
        .collect();
    items.sort_by_key(|(_, item)| std::cmp::Reverse(item.updated));
    let mut starting: Vec<&ServerKey> = starting
        .filter(|key| progress.get(*key).is_none_or(Vec::is_empty))
        .collect();
    starting.sort_by(|a, b| (&a.server, &a.root).cmp(&(&b.server, &b.root)));
    let first = match items.first() {
        Some((server, item)) => format!("{server}: {}", item.text()),
        None => format!("Starting {}…", starting.first()?.server),
    };
    let others = items.len() + starting.len() - 1;
    let mut text = truncate(&first, PROGRESS_TEXT_MAX);
    if others > 0 {
        text.push_str(&format!(" +{others}"));
    }
    Some(text)
}

/// `text`, or its first `max - 1` characters and `…` when longer.
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(max - 1).collect();
    cut.push('…');
    cut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_text() {
        let item = ProgressItem {
            updated: 1,
            token: "t".into(),
            title: Some("Indexing".into()),
            message: Some("crates/foo".into()),
            percentage: Some(45),
        };
        assert_eq!(item.text(), "Indexing 45% — crates/foo");
    }

    fn key(server: &str) -> ServerKey {
        ServerKey {
            root: Arc::from(Path::new("/p")),
            server: server.into(),
        }
    }

    fn item(updated: u64, title: &str) -> ProgressItem {
        ProgressItem {
            updated,
            token: title.into(),
            title: Some(title.into()),
            message: None,
            percentage: None,
        }
    }

    #[test]
    fn progress_text_counts_a_starting_server_with_progress_once() {
        let (a, b, c) = (key("a"), key("b"), key("c"));
        let mut progress = HashMap::new();
        progress.insert(a.clone(), vec![item(2, "Loading")]);
        // `a` is still starting but reports progress; `b` and `c` start.
        let text = summarize_progress(&progress, [&a, &b, &c].into_iter());
        assert_eq!(text.as_deref(), Some("a: Loading +2"));

        progress.insert(b.clone(), vec![item(3, "Indexing")]);
        let text = summarize_progress(&progress, [&a, &b].into_iter());
        assert_eq!(text.as_deref(), Some("b: Indexing +1"));

        progress.clear();
        let text = summarize_progress(&progress, [&c, &b].into_iter());
        assert_eq!(text.as_deref(), Some("Starting b… +1"));
        assert_eq!(summarize_progress(&progress, std::iter::empty()), None);
    }

    #[test]
    fn truncate_keeps_max_characters() {
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abcd", 3), "ab…");
        assert_eq!(truncate("ééééé", 4), "ééé…");
    }
}

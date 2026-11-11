//! A server's events on the UI thread: diagnostics (batched), progress,
//! logs, watch registrations and exit.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Weak};
use std::task::Poll;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use gpui::{Context, Task};
use lucerna_lsp::position::uri_to_path;
use lucerna_lsp::{LanguageServer, LogKind, LogLine, ServerEvent, ServerKey, Slot};

use super::{LanguageServers, LanguageServersEvent};
use crate::workspace_state::absolute_path;

/// Lines kept in each server's log.
const LOG_LINES: usize = 2000;
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
            ServerEvent::Log(line) => self.push_log(key.server, line.kind, line.text),
            ServerEvent::ShowMessage { message, .. } => {
                self.push_log(key.server, LogKind::Server, message);
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
                        token,
                        title: None,
                        message: None,
                        percentage: None,
                    },
                };
                if !done {
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

    pub(super) fn push_log(&mut self, server: &'static str, kind: LogKind, text: String) {
        let log = self.logs.entry(server).or_default();
        if log.len() == LOG_LINES {
            log.pop_front();
        }
        log.push_back(LogLine { kind, text });
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
                let paths = std::mem::take(&mut servers.changed_diagnostics);
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_text() {
        let item = ProgressItem {
            token: "t".into(),
            title: Some("Indexing".into()),
            message: Some("crates/foo".into()),
            percentage: Some(45),
        };
        assert_eq!(item.text(), "Indexing 45% — crates/foo");
    }
}

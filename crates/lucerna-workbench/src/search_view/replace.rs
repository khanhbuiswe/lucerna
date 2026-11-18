//! Replace All of the Search tab: closed files go to the background job
//! (`lucerna_project::replace_files`), files open in the editor are edited
//! through `Documents` as one undo step and saved only when they had no
//! unsaved changes before.
//!
//! Users' files are at stake and an agent may be writing them at the same
//! time: nothing here ever writes over a change made since the files were
//! read, saves a document the user was editing, or waits on the disk on the
//! UI thread.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::StreamExt;
use gpui::{App, AsyncWindowContext, Entity, Task, Window};
use lucerna_project::{
    FileOutcome, ReplaceProgress, ReplaceSpec, ShownLine, SkipReason, edits_for_text, replace_files,
};
use lucerna_text::WriteError;

use crate::DocState;
use crate::documents::{Documents, Edits};
use crate::find_widget::apply_replacements;
use crate::save::{self, SaveOptions, SaveReason};
use crate::tasks::guarded;

/// A result file and the match lines the results show for it.
pub(crate) struct ReplaceTarget {
    /// Relative to the root, as in the results.
    pub path: Arc<Path>,
    pub lines: Vec<ShownLine>,
}

/// What a Replace All did.
#[derive(Debug, Default)]
pub(crate) struct ReplaceReport {
    /// Occurrences replaced.
    pub replaced: usize,
    /// Files with at least one occurrence replaced.
    pub files: usize,
    /// Files left alone, by root-relative path, sorted.
    pub skipped: Vec<(String, SkipReason)>,
    /// Open documents replaced in their buffer only, because they had
    /// unsaved changes.
    pub unsaved: usize,
}

/// What became of one open document.
enum OpenOutcome {
    Skipped(SkipReason),
    /// Edited in the buffer; `save` is the save of a document that was
    /// clean before, `None` for one that was dirty.
    Replaced {
        count: usize,
        save: Option<Task<Result<bool, WriteError>>>,
    },
}

/// Never asks: an autosave's semantics (a conflict or a missing file is
/// left alone), and no format or trim on a file whose text was only
/// replaced.
const SAVE: SaveOptions = SaveOptions {
    reason: SaveReason::Auto,
    participants: false,
    overwrite: false,
};

/// Replaces the shown match lines of `targets`. Closed files are handled
/// by a background job; open documents one per foreground turn. Progress
/// (files done of all) goes to `on_progress`. Setting `cancel` stops the
/// job between files; a dropped task stops the open documents too.
#[allow(clippy::too_many_arguments)] // the job's inputs, then gpui's window and app
pub(crate) fn run_replace(
    targets: Vec<ReplaceTarget>,
    spec: ReplaceSpec,
    documents: Entity<Documents>,
    root: PathBuf,
    cancel: Arc<AtomicBool>,
    on_progress: impl Fn(ReplaceProgress, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) -> Task<ReplaceReport> {
    let total = targets.len();
    let mut report = ReplaceReport::default();
    let (mut open, mut closed) = (Vec::new(), Vec::new());
    {
        let docs = documents.read(cx);
        for target in targets {
            match docs.state(&target.path) {
                Some(DocState::Ready) if docs.snapshot_of(&target.path).is_some() => {
                    open.push(target);
                }
                Some(DocState::Loading | DocState::Error(_)) => {
                    report
                        .skipped
                        .push((display(&target.path), SkipReason::Unreadable));
                }
                _ => closed.push(target),
            }
        }
    }
    let spec = Arc::new(spec);
    let temps = save::save_temps(cx);
    let closed_files: Vec<(PathBuf, Vec<ShownLine>)> = closed
        .into_iter()
        .map(|t| (root.join(&t.path), t.lines))
        .collect();
    let closed_count = closed_files.len();
    let (progress, mut progress_rx) = futures::channel::mpsc::unbounded::<usize>();
    let job = {
        let (spec, cancel) = (spec.clone(), cancel.clone());
        cx.background_executor().spawn(async move {
            let paths: Vec<PathBuf> = closed_files.iter().map(|(p, _)| p.clone()).collect();
            // `replace_files` blocks and reports from its worker threads:
            // it runs on its own thread while this task forwards the
            // progress, so the foreground is only woken from here.
            let (sink, events) = crossbeam_channel::unbounded::<usize>();
            let outcomes = std::thread::scope(|scope| {
                let (spec, temps, cancel) = (&spec, &temps, &cancel);
                let job = scope.spawn(move || {
                    let report = |p: ReplaceProgress| {
                        sink.send(p.done).ok();
                    };
                    guarded("replace", || {
                        replace_files(closed_files, spec, temps, cancel, &report)
                    })
                });
                for done in events {
                    progress.unbounded_send(done).ok();
                }
                job.join().ok().flatten()
            });
            outcomes.unwrap_or_else(|| {
                // The job panicked: which files it wrote is unknown.
                paths
                    .into_iter()
                    .map(|p| {
                        let reason = SkipReason::Unwritable("the replace job failed".into());
                        (p, FileOutcome::Skipped(reason))
                    })
                    .collect()
            })
        })
    };
    window.spawn(cx, async move |cx| {
        let mut done_open = report.skipped.len();
        let mut done_closed = 0;
        let send = |done: usize, cx: &mut AsyncWindowContext| {
            cx.update(|_, cx| on_progress(ReplaceProgress { done, total }, cx))
                .ok();
        };
        let mut saves = Vec::new();
        for target in open {
            if cancel.load(Ordering::SeqCst) {
                break;
            }
            let outcome = replace_open(&target, &spec, &documents, cx).await;
            match outcome {
                OpenOutcome::Skipped(reason) => {
                    report.skipped.push((display(&target.path), reason))
                }
                OpenOutcome::Replaced { count, save } => match save {
                    Some(save) => saves.push((target.path.clone(), count, save)),
                    None => {
                        if count > 0 {
                            report.replaced += count;
                            report.files += 1;
                            report.unsaved += 1;
                        }
                    }
                },
            }
            done_open += 1;
            while let Ok(done) = progress_rx.try_recv() {
                done_closed = done;
            }
            send(done_open + done_closed, cx);
        }
        // The closed files' progress, in batches, until the job ends.
        while let Some(first) = progress_rx.next().await {
            done_closed = first;
            while let Ok(done) = progress_rx.try_recv() {
                done_closed = done;
            }
            send(done_open + done_closed, cx);
        }
        let outcomes = job.await;
        debug_assert!(outcomes.len() <= closed_count);
        for (path, outcome) in outcomes {
            let path = path
                .strip_prefix(&root)
                .map_or(path.clone(), Path::to_path_buf);
            match outcome {
                FileOutcome::Replaced(0) => {}
                FileOutcome::Replaced(n) => {
                    report.replaced += n;
                    report.files += 1;
                }
                FileOutcome::Skipped(reason) => report.skipped.push((display(&path), reason)),
            }
        }
        for (path, count, save) in saves {
            match finish_save(&path, save.await, &documents, &root, cx).await {
                None => {
                    report.replaced += count;
                    report.files += 1;
                }
                Some(reason) => report.skipped.push((display(&path), reason)),
            }
        }
        report.skipped.sort_by(|a, b| a.0.cmp(&b.0));
        report
    })
}

/// How a save of an edited document ended: `None` when it was written,
/// else why not. The edit stays in the buffer either way.
async fn finish_save(
    path: &Path,
    result: Result<bool, WriteError>,
    documents: &Entity<Documents>,
    root: &Path,
    cx: &mut AsyncWindowContext,
) -> Option<SkipReason> {
    match result {
        Ok(true) => None,
        Ok(false) => {
            let conflict = cx
                .update(|_, cx| documents.read(cx).has_conflict(path))
                .unwrap_or(false);
            if conflict {
                return Some(SkipReason::ChangedOnDisk);
            }
            let absolute = root.join(path);
            let exists = cx
                .background_executor()
                .spawn(async move { absolute.exists() })
                .await;
            Some(if exists {
                // Not written for another reason (the file was reloaded
                // from disk meanwhile): nothing was saved.
                SkipReason::ChangedOnDisk
            } else {
                SkipReason::Missing
            })
        }
        Err(err) => Some(SkipReason::Unwritable(err.to_string())),
    }
}

/// Why the open document at `path` cannot be replaced now, if it cannot.
fn open_skip(documents: &Documents, path: &Path) -> Option<SkipReason> {
    if documents.state(path) != Some(&DocState::Ready) {
        return Some(SkipReason::Unreadable);
    }
    if documents.has_conflict(path) {
        return Some(SkipReason::Conflict);
    }
    if documents.is_lossy(path) {
        return Some(SkipReason::Lossy);
    }
    None
}

/// Replaces the shown lines of one open document: the edits are worked
/// out on a background thread from a snapshot, and applied only if the
/// text did not change meanwhile (worked out once more if it did).
async fn replace_open(
    target: &ReplaceTarget,
    spec: &Arc<ReplaceSpec>,
    documents: &Entity<Documents>,
    cx: &mut AsyncWindowContext,
) -> OpenOutcome {
    let lines = Arc::new(target.lines.clone());
    let path = target.path.clone();
    for _ in 0..2 {
        let taken = cx.update(|_, cx| {
            let docs = documents.read(cx);
            if let Some(reason) = open_skip(docs, &path) {
                return Err(reason);
            }
            match (docs.snapshot_of(&path), docs.revision_of(&path)) {
                (Some(snapshot), Some(revision)) => Ok((snapshot, revision)),
                _ => Err(SkipReason::Unreadable),
            }
        });
        let (snapshot, revision) = match taken {
            Ok(Ok(taken)) => taken,
            Ok(Err(reason)) => return OpenOutcome::Skipped(reason),
            Err(_) => return OpenOutcome::Skipped(SkipReason::ChangedWhileReplacing),
        };
        let edits = {
            let (lines, spec) = (lines.clone(), spec.clone());
            cx.background_executor()
                .spawn(async move { edits_for_text(&snapshot.text(), &lines, &spec) })
                .await
        };
        let edits = match edits {
            Ok(edits) => edits,
            Err(reason) => return OpenOutcome::Skipped(reason),
        };
        let applied = cx.update(|window, cx| {
            let docs = documents.read(cx);
            if docs.revision_of(&path) != Some(revision) {
                return None;
            }
            if let Some(reason) = open_skip(docs, &path) {
                return Some(OpenOutcome::Skipped(reason));
            }
            let count = edits.len();
            if count == 0 {
                return Some(OpenOutcome::Replaced { count, save: None });
            }
            if !sorted_apart(&edits) {
                return Some(OpenOutcome::Skipped(SkipReason::ChangedSinceSearch));
            }
            let was_dirty = docs.is_dirty(&path);
            apply(documents, &path, edits, cx);
            let save = (!was_dirty)
                .then(|| documents.update(cx, |docs, cx| docs.save(&path, SAVE, window, cx)));
            Some(OpenOutcome::Replaced { count, save })
        });
        match applied {
            Ok(Some(outcome)) => return outcome,
            // The text changed meanwhile: work the edits out again, once.
            Ok(None) => {}
            Err(_) => return OpenOutcome::Skipped(SkipReason::ChangedWhileReplacing),
        }
    }
    OpenOutcome::Skipped(SkipReason::ChangedWhileReplacing)
}

/// The edits are sorted and do not overlap, as `apply_edits` needs.
fn sorted_apart(edits: &[(std::ops::Range<usize>, String)]) -> bool {
    edits
        .windows(2)
        .all(|pair| pair[0].0.end <= pair[1].0.start)
}

/// Applies `edits` to the document at `path` as one undo step. Through a
/// view when there is one that takes edits, so its selections move
/// through them as the find bar's Replace All does; the document's other
/// views follow as for any edit.
fn apply(
    documents: &Entity<Documents>,
    path: &Path,
    edits: Vec<(std::ops::Range<usize>, String)>,
    cx: &mut App,
) {
    // Stops on the document itself, around the step: a view in vim's
    // Insert session pushes none (`suppress_stops`), and the replace must
    // never merge with the user's typing on either side.
    documents.update(cx, |docs, _| docs.push_stop(path));
    let view = documents
        .read(cx)
        .views_of(path)
        .into_iter()
        .find(|view| !view.read(cx).is_readonly());
    let applied = view.is_some_and(|view| apply_replacements(&view, edits.clone(), cx));
    if !applied {
        let edits: Edits = edits
            .into_iter()
            .map(|(range, text)| (range, Arc::from(text)))
            .collect();
        documents.update(cx, |docs, cx| {
            let origin = cx.entity_id();
            docs.apply_edits(path, edits, None, None, origin, cx);
        });
    }
    documents.update(cx, |docs, _| docs.push_stop(path));
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

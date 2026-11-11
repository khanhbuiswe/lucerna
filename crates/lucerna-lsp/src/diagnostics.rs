//! Diagnostics published by servers, kept per file and server.
//!
//! Entries for open files are converted to [`Point`]s at publish time.
//! Entries for unopened files stay raw (LSP ranges) until something needs
//! points; then the file is read from disk, so [`DiagnosticStore::for_file`]
//! and [`DiagnosticStore::next`] can block on file reads for unopened files:
//! call them off the UI thread, or only for open files.

use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use lsp_types::{DiagnosticSeverity, DiagnosticTag, NumberOrString};
use lucerna_text::{BufferSnapshot, Point};

use crate::locations::read_lines_snapshot;
use crate::position::{PositionEncoding, range_from_lsp, uri_to_path};
use crate::registry::ServerKey;

/// `Error` sorts first.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub range: Range<Point>,
    pub severity: Severity,
    pub message: String,
    pub source: Option<String>,
    pub code: Option<String>,
    pub code_href: Option<String>,
    pub unnecessary: bool,
    pub deprecated: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    File,
    Files,
}

#[derive(Clone)]
enum Entry {
    Converted(Arc<[Diagnostic]>),
    Raw {
        diagnostics: Arc<[lsp_types::Diagnostic]>,
        encoding: PositionEncoding,
    },
}

/// Cloning is cheap (entries are shared): a copy can go to a background
/// thread for [`DiagnosticStore::next`].
#[derive(Default, Clone)]
pub struct DiagnosticStore {
    files: BTreeMap<PathBuf, HashMap<ServerKey, Entry>>,
}

fn severity(raw: Option<DiagnosticSeverity>) -> Severity {
    match raw {
        Some(DiagnosticSeverity::WARNING) => Severity::Warning,
        Some(DiagnosticSeverity::INFORMATION) => Severity::Information,
        Some(DiagnosticSeverity::HINT) => Severity::Hint,
        _ => Severity::Error,
    }
}

fn convert(
    raw: &lsp_types::Diagnostic,
    snapshot: &BufferSnapshot,
    encoding: PositionEncoding,
) -> Diagnostic {
    let has_tag = |tag: DiagnosticTag| raw.tags.as_ref().is_some_and(|tags| tags.contains(&tag));
    Diagnostic {
        range: range_from_lsp(snapshot, raw.range, encoding),
        severity: severity(raw.severity),
        message: raw.message.clone(),
        source: raw.source.clone(),
        code: raw.code.as_ref().map(|code| match code {
            NumberOrString::Number(n) => n.to_string(),
            NumberOrString::String(s) => s.clone(),
        }),
        code_href: raw
            .code_description
            .as_ref()
            .map(|description| description.href.as_str().to_owned()),
        unnecessary: has_tag(DiagnosticTag::UNNECESSARY),
        deprecated: has_tag(DiagnosticTag::DEPRECATED),
    }
}

fn convert_all(
    raw: &[lsp_types::Diagnostic],
    snapshot: &BufferSnapshot,
    encoding: PositionEncoding,
) -> Vec<Diagnostic> {
    raw.iter().map(|d| convert(d, snapshot, encoding)).collect()
}

fn start_key(d: &Diagnostic) -> (u32, u32) {
    (d.range.start.row, d.range.start.column)
}

/// The total order of a file's diagnostics: ties on the start are broken
/// so the order does not depend on which server published first.
fn order(a: &Diagnostic, b: &Diagnostic) -> std::cmp::Ordering {
    let end = |d: &Diagnostic| (d.range.end.row, d.range.end.column);
    (start_key(a), end(a), a.severity, &a.message).cmp(&(
        start_key(b),
        end(b),
        b.severity,
        &b.message,
    ))
}

impl Entry {
    fn is_empty(&self) -> bool {
        match self {
            Entry::Converted(v) => v.is_empty(),
            Entry::Raw { diagnostics, .. } => diagnostics.is_empty(),
        }
    }

    /// Whether any diagnostic is not a hint; known without converting.
    fn has_non_hint(&self) -> bool {
        match self {
            Entry::Converted(v) => v.iter().any(|d| d.severity != Severity::Hint),
            Entry::Raw { diagnostics, .. } => diagnostics
                .iter()
                .any(|d| severity(d.severity) != Severity::Hint),
        }
    }
}

impl DiagnosticStore {
    /// Replaces the diagnostics of one file for one server. `open` is the
    /// file's snapshot and the version the editor has synced to this server
    /// (see `DocumentSync::version`), `None` when the file is not open.
    /// Returns `None` when the publish is dropped as stale: it names a
    /// version other than the current one. A publish without a version is
    /// always accepted. Non-file URIs are ignored.
    pub fn publish(
        &mut self,
        key: &ServerKey,
        params: lsp_types::PublishDiagnosticsParams,
        encoding: PositionEncoding,
        open: Option<(&BufferSnapshot, i32)>,
    ) -> Option<PathBuf> {
        let path = uri_to_path(&params.uri)?;
        if let (Some(version), Some((_, current))) = (params.version, open)
            && version != current
        {
            return None;
        }
        let entry = match open {
            Some((snapshot, _)) => {
                Entry::Converted(convert_all(&params.diagnostics, snapshot, encoding).into())
            }
            None => Entry::Raw {
                diagnostics: params.diagnostics.into(),
                encoding,
            },
        };
        if entry.is_empty() {
            if let Some(servers) = self.files.get_mut(&path) {
                servers.remove(key);
                if servers.is_empty() {
                    self.files.remove(&path);
                }
            }
        } else {
            self.files
                .entry(path.clone())
                .or_default()
                .insert(key.clone(), entry);
        }
        Some(path)
    }

    /// Converts the raw entries of a file that was just opened or reloaded.
    pub fn resolve_open(&mut self, path: &Path, snapshot: &BufferSnapshot) {
        let Some(servers) = self.files.get_mut(path) else {
            return;
        };
        for entry in servers.values_mut() {
            if let Entry::Raw {
                diagnostics,
                encoding,
            } = entry
            {
                *entry = Entry::Converted(convert_all(diagnostics, snapshot, *encoding).into());
            }
        }
    }

    /// The diagnostics of all servers for a file, sorted by range start.
    /// Raw entries of an unopened file are converted by reading it from
    /// disk (blocking); they are skipped when it cannot be read.
    pub fn for_file(&self, path: &Path) -> Vec<Diagnostic> {
        self.for_file_with_reader(path, &read_lines_snapshot)
    }

    fn for_file_with_reader(
        &self,
        path: &Path,
        read: &dyn Fn(&Path) -> Option<BufferSnapshot>,
    ) -> Vec<Diagnostic> {
        let Some(servers) = self.files.get(path) else {
            return Vec::new();
        };
        let mut snapshot = None;
        let mut out = Vec::new();
        for entry in servers.values() {
            match entry {
                Entry::Converted(diagnostics) => out.extend_from_slice(diagnostics),
                Entry::Raw {
                    diagnostics,
                    encoding,
                } => {
                    if snapshot.is_none() {
                        snapshot = Some(read(path));
                    }
                    if let Some(Some(snapshot)) = &snapshot {
                        out.extend(convert_all(diagnostics, snapshot, *encoding));
                    }
                }
            }
        }
        out.sort_by(order);
        out
    }

    /// `(errors, warnings)` over every file, open or not. Never reads files.
    pub fn summary(&self) -> (usize, usize) {
        let (mut errors, mut warnings) = (0, 0);
        let mut count = |s: Severity| match s {
            Severity::Error => errors += 1,
            Severity::Warning => warnings += 1,
            _ => {}
        };
        for entry in self.files.values().flat_map(HashMap::values) {
            match entry {
                Entry::Converted(diagnostics) => {
                    diagnostics.iter().for_each(|d| count(d.severity));
                }
                Entry::Raw { diagnostics, .. } => {
                    diagnostics.iter().for_each(|d| count(severity(d.severity)));
                }
            }
        }
        (errors, warnings)
    }

    /// Forgets everything a server published; the files that had entries.
    pub fn clear_server(&mut self, key: &ServerKey) -> Vec<PathBuf> {
        let mut cleared = Vec::new();
        self.files.retain(|path, servers| {
            if servers.remove(key).is_some() {
                cleared.push(path.clone());
            }
            !servers.is_empty()
        });
        cleared
    }

    /// The diagnostic strictly after (or before) `from`, skipping hints.
    /// Files are ordered by path, diagnostics by (start, end, severity,
    /// message). Wraps around; `None` when there is nothing else. May read
    /// unopened files (blocking): see the module docs.
    ///
    /// Several diagnostics can share a start. A point origin cannot tell
    /// them apart, so stepping goes past the whole group: forward lands on
    /// the first of a group, backward on the last.
    pub fn next(
        &self,
        from: (&Path, Point),
        forward: bool,
        scope: Scope,
    ) -> Option<(PathBuf, Diagnostic)> {
        self.next_with_reader(from, forward, scope, &read_lines_snapshot)
    }

    /// [`DiagnosticStore::next`] with the way unopened files are read.
    /// Lazy: files are walked in path order from the origin, then wrapped;
    /// a file whose raw entries are all hints is skipped unread, and the
    /// walk stops at the first file with a hit.
    pub fn next_with_reader(
        &self,
        from: (&Path, Point),
        forward: bool,
        scope: Scope,
        read: &dyn Fn(&Path) -> Option<BufferSnapshot>,
    ) -> Option<(PathBuf, Diagnostic)> {
        use std::ops::Bound::{Excluded, Unbounded};
        let (from_path, from_point) = from;
        let from_pos = (from_point.row, from_point.column);
        // The first (or last) match of a file in walk order.
        let pick = |path: &Path| -> Option<Vec<Diagnostic>> {
            let servers = self.files.get(path)?;
            if !servers.values().any(Entry::has_non_hint) {
                return None;
            }
            let mut diagnostics = self.for_file_with_reader(path, read);
            diagnostics.retain(|d| d.severity != Severity::Hint);
            (!diagnostics.is_empty()).then_some(diagnostics)
        };
        let hit = |path: &Path, d: &Diagnostic| Some((path.to_path_buf(), d.clone()));
        let origin = pick(from_path);
        let after = |diagnostics: &[Diagnostic]| -> Option<Diagnostic> {
            if forward {
                diagnostics
                    .iter()
                    .find(|d| start_key(d) > from_pos)
                    .cloned()
            } else {
                diagnostics
                    .iter()
                    .rev()
                    .find(|d| start_key(d) < from_pos)
                    .cloned()
            }
        };
        if let Some(d) = origin.as_deref().and_then(after) {
            return hit(from_path, &d);
        }
        if scope == Scope::Files {
            let ahead: Vec<&PathBuf> = if forward {
                self.files
                    .range::<Path, _>((Excluded(from_path), Unbounded))
                    .map(|(p, _)| p)
                    .collect()
            } else {
                self.files
                    .range::<Path, _>((Unbounded, Excluded(from_path)))
                    .map(|(p, _)| p)
                    .rev()
                    .collect()
            };
            let behind: Vec<&PathBuf> = if forward {
                self.files
                    .range::<Path, _>((Unbounded, Excluded(from_path)))
                    .map(|(p, _)| p)
                    .collect()
            } else {
                self.files
                    .range::<Path, _>((Excluded(from_path), Unbounded))
                    .map(|(p, _)| p)
                    .rev()
                    .collect()
            };
            // Past the origin in walk order, then wrapped around.
            for path in ahead.into_iter().chain(behind) {
                if let Some(diagnostics) = pick(path) {
                    let d = if forward {
                        diagnostics.first()
                    } else {
                        diagnostics.last()
                    };
                    return d.and_then(|d| hit(path, d));
                }
            }
        }
        // Wrapped all the way back into the origin file.
        let diagnostics = origin?;
        let wrapped = if forward {
            diagnostics.iter().find(|d| start_key(d) < from_pos)
        } else {
            diagnostics.iter().rev().find(|d| start_key(d) > from_pos)
        };
        wrapped.and_then(|d| hit(from_path, d))
    }
}

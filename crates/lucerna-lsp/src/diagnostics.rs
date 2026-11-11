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

enum Entry {
    Converted(Vec<Diagnostic>),
    Raw {
        diagnostics: Vec<lsp_types::Diagnostic>,
        encoding: PositionEncoding,
    },
}

#[derive(Default)]
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

impl Entry {
    fn is_empty(&self) -> bool {
        match self {
            Entry::Converted(v) => v.is_empty(),
            Entry::Raw { diagnostics, .. } => diagnostics.is_empty(),
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
                Entry::Converted(convert_all(&params.diagnostics, snapshot, encoding))
            }
            None => Entry::Raw {
                diagnostics: params.diagnostics,
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
                *entry = Entry::Converted(convert_all(diagnostics, snapshot, *encoding));
            }
        }
    }

    /// The diagnostics of all servers for a file, sorted by range start.
    /// Raw entries of an unopened file are converted by reading it from
    /// disk (blocking); they are skipped when it cannot be read.
    pub fn for_file(&self, path: &Path) -> Vec<Diagnostic> {
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
                        snapshot = Some(read_lines_snapshot(path));
                    }
                    if let Some(Some(snapshot)) = &snapshot {
                        out.extend(convert_all(diagnostics, snapshot, *encoding));
                    }
                }
            }
        }
        out.sort_by_key(start_key);
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
    /// Files are ordered by path, diagnostics by range start. Wraps around;
    /// `None` when there is nothing else. May read unopened files
    /// (blocking): see the module docs.
    pub fn next(
        &self,
        from: (&Path, Point),
        forward: bool,
        scope: Scope,
    ) -> Option<(PathBuf, Diagnostic)> {
        let (from_path, from_point) = from;
        let from_pos = (from_point.row, from_point.column);
        let paths: Vec<&PathBuf> = match scope {
            Scope::File => self
                .files
                .get_key_value(from_path)
                .map(|(path, _)| path)
                .into_iter()
                .collect(),
            Scope::Files => self.files.keys().collect(),
        };
        let mut all = Vec::new();
        for path in paths {
            all.extend(
                self.for_file(path)
                    .into_iter()
                    .filter(|d| d.severity != Severity::Hint)
                    .map(|d| (path, d)),
            );
        }
        // `for_file` sorts by start and the paths are in order: `all` is
        // already ordered by (path, start).
        let order = |(path, d): &(&PathBuf, Diagnostic)| {
            (path.as_path().cmp(from_path), start_key(d).cmp(&from_pos))
        };
        let side = |entry: &(&PathBuf, Diagnostic)| {
            let (by_path, by_pos) = order(entry);
            by_path.then(by_pos)
        };
        let chosen = if forward {
            all.iter()
                .find(|e| side(e).is_gt())
                .or_else(|| all.iter().find(|e| side(e).is_ne()))
        } else {
            all.iter()
                .rev()
                .find(|e| side(e).is_lt())
                .or_else(|| all.iter().rev().find(|e| side(e).is_ne()))
        };
        chosen.map(|(path, d)| ((*path).clone(), d.clone()))
    }
}

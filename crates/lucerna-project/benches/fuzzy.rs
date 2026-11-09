//! `fuzzy_80k`: one Quick Open query over an 80,000-file snapshot.
//! Builds the snapshot in memory, so it needs no repo on disk.

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use lucerna_project::{Entry, EntryKind, QUICK_OPEN_LIMIT, WorktreeSnapshot, match_files};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// Same shape as `scripts/gen-bench-repo.sh`: 4,000 dirs x 20 files.
fn snapshot() -> WorktreeSnapshot {
    let mut entries = Vec::with_capacity(84_100);
    for pkg in 0..40 {
        entries.push(dir(format!("pkg{pkg:02}")));
    }
    for d in 0..4000 {
        let dir_path = format!("pkg{:02}/mod{d:04}", d / 100);
        entries.push(dir(dir_path.clone()));
        for f in 0..20 {
            let ext = if f % 2 == 0 { "rs" } else { "ts" };
            let path = format!("{dir_path}/item{f:02}.{ext}");
            entries.push(Entry {
                path: Arc::from(Path::new(&path)),
                kind: EntryKind::File,
            });
        }
    }
    WorktreeSnapshot::from_entries(entries)
}

fn dir(path: String) -> Entry {
    Entry {
        path: Arc::from(Path::new(&path)),
        kind: EntryKind::Dir,
    }
}

fn fuzzy_80k(c: &mut Criterion) {
    let snapshot = snapshot();
    let cancel = AtomicBool::new(false);
    let mut group = c.benchmark_group("fuzzy_80k");
    group.sample_size(20);
    for query in ["item07", "p12m1234i05ts"] {
        group.bench_function(query, |b| {
            b.iter(|| {
                black_box(match_files(
                    &snapshot,
                    black_box(query),
                    &[],
                    QUICK_OPEN_LIMIT,
                    &cancel,
                ))
            });
        });
    }
    group.finish();
}

criterion_group!(benches, fuzzy_80k);
criterion_main!(benches);

//! `scan_80k`: time from `Worktree::start` until `ScanComplete`.
//! Needs `LUCERNA_BENCH_REPO` (see `scripts/gen-bench-repo.sh`).

use criterion::{Criterion, criterion_group, criterion_main};
use lucerna_project::{ScanOptions, Worktree, WorktreeEvent};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn scan_80k(c: &mut Criterion) {
    let Some(root) = std::env::var_os("LUCERNA_BENCH_REPO").map(PathBuf::from) else {
        eprintln!("LUCERNA_BENCH_REPO not set; skipping scan_80k");
        return;
    };
    let mut group = c.benchmark_group("scan_80k");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(15));
    group.bench_function("scan", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let start = Instant::now();
                let worktree = Worktree::start(root.clone(), ScanOptions::default());
                let events = worktree.events();
                while let Ok(event) = events.recv() {
                    if event == WorktreeEvent::ScanComplete {
                        break;
                    }
                }
                total += start.elapsed();
                drop(worktree);
            }
            total
        });
    });
    group.finish();
}

criterion_group!(benches, scan_80k);
criterion_main!(benches);

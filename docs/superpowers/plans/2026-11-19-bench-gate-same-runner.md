# Bench gate on one runner Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The CI `bench` job measures a PR's base commit and the PR commit in the same job and gates on the difference.

**Architecture:** One POSIX shell script lists the gated benches and runs them in `save` or `compare` mode, skipping any bench whose source file is missing. The workflow copies that script out of the tree, runs it in `save` mode on the base SHA, checks the PR commit back out, runs `compare`, then `bench-gate.sh`.

**Tech Stack:** GitHub Actions (`macos-15`), POSIX `sh`, criterion via `cargo bench`.

**Spec:** `docs/superpowers/specs/2026-11-19-bench-gate-same-runner-design.md`

## Global Constraints

- Gate threshold stays `15` percent; `scripts/bench-gate.sh` is unchanged.
- Gated benches stay exactly: `lucerna-project` scan + fuzzy, `lucerna-syntax` highlight, `lucerna-text` reload. Never build gpui-dependent crates (`lucerna-editor`'s `typing` is not gated).
- Scripts are POSIX `sh` with `set -eu`, like the other files in `scripts/`; must pass `shellcheck`.
- Work on branch `claude/bench-same-runner` in a worktree from `origin/main` (the shared checkout's `main` is behind origin; never rebase it). Stage by explicit path; trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Locally, never set `CARGO_TARGET_DIR`; the plan needs no local `cargo bench` run.

## Review Focus

- The base commit has no `scripts/bench-run.sh` (it only exists after this lands): the workflow must run a copy taken from the PR before checking out the base — pinned by Task 2 Step 3's ordering and the first PR's run.
- A PR adds a new bench file: the base run skips it, the compare run runs it, the gate prints `NEW (no baseline)` — pinned by Task 1's missing-file test.
- Checkout depth: `base.sha` must be reachable; a shallow checkout fails `git checkout` — pinned by `fetch-depth: 0` in Task 2.
- Push to `main` no longer runs `bench` at all, so no runner minutes are spent there — pinned by the job-level `if` in Task 2.
- A tests-only PR must pass the gate — pinned by Task 2's end-to-end PR check.

---

### Task 1: `scripts/bench-run.sh`

**Files:**
- Create: `scripts/bench-run.sh`
- Test: `scripts/test-bench-run.sh`

**Interfaces:**
- Produces: `scripts/bench-run.sh <save|compare>`, run from the repo root. `save` → `cargo bench -p <crate> --bench <name> -- --save-baseline main`; `compare` → `... -- --baseline-lenient main` (strict `--baseline` panics on a bench with no saved baseline). One `cargo bench` call per bench; skips (prints `skip <crate>/<name>: no benches/<name>.rs`) when `crates/<crate>/benches/<name>.rs` is missing. With `BENCH_DRY_RUN=1` it prints each command (`cargo bench -p ...`) instead of running it. Any other argument → usage to stderr, exit 2.

- [ ] **Step 1: Write the failing test** `scripts/test-bench-run.sh` (sh, `set -eu`), which builds a temp tree under `mktemp -d` with `crates/lucerna-project/benches/{scan,fuzzy}.rs`, `crates/lucerna-syntax/benches/highlight.rs`, and no `lucerna-text` bench, `cd`s into it and runs the script by absolute path with `BENCH_DRY_RUN=1`. Asserts:
  - `save` output contains exactly these three lines, in this order:
    `cargo bench -p lucerna-project --bench scan -- --save-baseline main`
    `cargo bench -p lucerna-project --bench fuzzy -- --save-baseline main`
    `cargo bench -p lucerna-syntax --bench highlight -- --save-baseline main`
    plus the line `skip lucerna-text/reload: no benches/reload.rs`.
  - `compare` prints the same three with `--baseline-lenient main`.
  - `bogus` exits 2.
  Prints `ok` and exits 0 when all pass.

- [ ] **Step 2: Run it to verify it fails**
Run: `sh scripts/test-bench-run.sh`
Expected: fails (script not found).

- [ ] **Step 3: Implement `scripts/bench-run.sh`** with a header comment like the other scripts, the four `crate:bench` pairs listed once, and the behaviour from Interfaces.

- [ ] **Step 4: Verify**
Run: `chmod +x scripts/bench-run.sh scripts/test-bench-run.sh && sh scripts/test-bench-run.sh && shellcheck scripts/bench-run.sh scripts/test-bench-run.sh`
Expected: `ok`, no shellcheck output.

- [ ] **Step 5: Commit**
```bash
git add scripts/bench-run.sh scripts/test-bench-run.sh
git commit -m "ci: one script lists and runs the gated benches"
```

### Task 2: Workflow measures base and head in one job

**Files:**
- Modify: `.github/workflows/ci.yml` (the `bench` job, lines 32–70)
- Add to commit: `docs/superpowers/specs/2026-11-19-bench-gate-same-runner-design.md`, this plan

**Interfaces:**
- Consumes: `scripts/bench-run.sh <save|compare>` (Task 1), `scripts/bench-gate.sh target/criterion 15` (existing).

- [ ] **Step 1: Job-level gate.** Add `if: github.event_name == 'pull_request'` to the `bench` job; delete the `Restore main baseline`, `Save main baseline`, `Cache main baseline` and `Compare with main baseline` steps.

- [ ] **Step 2: Checkout** with `fetch-depth: 0`. Keep toolchain, `Swatinem/rust-cache@v2` and `Generate bench repo` steps as they are.

- [ ] **Step 3: New steps, in this order:**
  1. `Measure base`: `cp scripts/bench-run.sh "$RUNNER_TEMP/bench-run.sh"`, `git checkout --detach ${{ github.event.pull_request.base.sha }}`, `sh "$RUNNER_TEMP/bench-run.sh" save`.
  2. `Measure PR`: `git checkout --detach ${{ github.sha }}`, `scripts/bench-run.sh compare`, `scripts/bench-gate.sh target/criterion 15`.
  Comment above them: both runs share one runner so machine speed cancels out.

- [ ] **Step 4: Verify locally**
Run: `actionlint .github/workflows/ci.yml`
Expected: no output.

- [ ] **Step 5: Commit**
```bash
git add .github/workflows/ci.yml docs/superpowers/specs/2026-11-19-bench-gate-same-runner-design.md docs/superpowers/plans/2026-11-19-bench-gate-same-runner.md
git commit -m "ci: compare benches with the PR base on the same runner"
```

- [ ] **Step 6: End-to-end check (ask the user before pushing).** Push the branch and open a PR; this PR changes no Rust code, so it is itself the tests-only case. Expected in the `bench` log: base run saves four baselines, compare run prints four `ok` lines from `bench-gate.sh`, job passes. If it fails on noise above 15% with both runs on one machine, report the numbers to the user instead of raising the threshold.

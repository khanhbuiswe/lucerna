# English-only Repository Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every file in the repository is English; Vietnamese stays only as test data in code, and the rule is written down for later sessions.

**Architecture:** A small checker script (`scripts/check-english.py`) finds Vietnamese lines and compares a file's Markdown structure before and after translation. The Markdown files are then translated in place in independent batches (one task each, parallelizable), each verified by the checker and committed alone. A last task fixes the code comments that quote Vietnamese headings, writes the rule, and runs the full scan.

**Tech Stack:** Python 3 standard library (checker + `unittest`), Markdown.

**Spec:** `docs/superpowers/specs/2026-11-21-english-only-repo-design.md`

## Global Constraints

- Translate, don't edit: no meaning changed, no detail dropped, nothing "improved".
- Keep every heading's level, number and order; code cites spec sections by number (`spec §7.2`).
- Never translate inside code blocks or inline code, except Vietnamese comments inside code blocks.
- Text the app shows is already English in the docs; keep it verbatim.
- Same-file anchor links (`#bổ-sung-...`) are updated to the slug of the translated heading.
- Plain, concise technical English matching `README.md` and the English parts of `docs/benchmarks.md`; US spelling.
- Status lines translate literally ("Bản nháp chờ duyệt" → "Draft, awaiting approval"; "Đã xong" → "Done").
- File names do not change. Git history is not rewritten.
- Vietnamese test data in `crates/**` stays.
- Shared checkout: stage by explicit path (`git add <paths>`, `git commit -- <paths>`), never `git add -A`; never rebase main. Before a batch, run `git status --short -- <batch files>`; if another session has uncommitted changes in one of them, skip that file and report it.
- Commit trailer: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. A table whose cells contain `|` inside inline code: translation must not add or drop columns. → the `structure` check counts table rows *and* pipes per row (Task 1 test `test_structure_detects_table_column_change`).
2. A heading whose number is written as `§3.1`, `3.1.` or `V2a` rather than `3.1`: the number must survive. → `test_heading_numbers_kept_for_section_and_alnum_forms` (Task 1).
3. A line that mixes Vietnamese prose and Vietnamese test data in backticks (`` `"xin chào"` `` in a spec): the prose is translated, the backticked text is kept. → the inline-code count in `structure` must match; scan allows Vietnamese inside backticks only in `crates/**` (`test_scan_flags_backticked_vietnamese_in_docs`).
4. A file another session edits while a batch runs (`docs/roadmap.md` changes often): the batch must not overwrite it. → the skip rule in Global Constraints; each batch's Step 1 checks it.
5. A Vietnamese heading slug referenced from the same file: the link must still resolve. → `structure` reports same-file anchors that match no heading (`test_structure_reports_broken_anchor`).

---

### Task 1: Checker script

**Files:**
- Create: `scripts/check-english.py`
- Test: `scripts/test_check_english.py`
- Modify: `.github/workflows/ci.yml` (job `check`, a step right after `actions/checkout`; this workflow runs on every push and PR with no path filter)

**Interfaces:**
- Produces:
  - `has_vietnamese(text: str) -> bool` — true if `text` contains any of the Vietnamese letters `àáạảãâầấậẩẫăằắặẳẵèéẹẻẽêềếệểễìíịỉĩòóọỏõôồốộổỗơờớợởỡùúụủũưừứựửữỳýỵỷỹđ` (case-insensitive). Note `é`, `è`, `ê`, `à`, `ô` etc. also occur in French/German test words; that is acceptable because they are allowed in `crates/**` literals anyway.
  - `scan_line(path: str, line: str) -> bool` — true if the line must be flagged. In `crates/**`, `bench/**` and `scripts/test_*.py`, Vietnamese inside `"..."`, `'...'` or `` `...` `` is allowed; any outside them is flagged. Everywhere else any Vietnamese is flagged.
  - `signature(markdown: str) -> dict` with keys `headings` (list of `(level, number)` where `number` is the leading token matching `§?[0-9A-Za-z]+(\.[0-9A-Za-z]+)*\.?` only when it contains a digit, else `None`), `fences` (count of lines starting with ```` ``` ````), `table_rows` (list of pipe counts per line starting with `|`, pipes inside backticks excluded), `inline_code` (count of inline code spans outside fences), `links` (count of `](`), `broken_anchors` (list of `#slug` targets in `](#...)` that match no heading's GitHub slug).
  - CLI: `python3 scripts/check-english.py scan [PATH...]` — prints `path:line: text` for flagged lines in the given paths (default: all `git ls-files`), exits 1 if any. `python3 scripts/check-english.py structure --base REV FILE...` — for each file compares `signature(file at REV)` with `signature(working tree)` on `headings` (levels and numbers), `fences`, `table_rows`, `inline_code`, `links`; prints each difference; also prints any `broken_anchors` in the working tree; exits 1 if anything was printed.
  - GitHub slug rule: lowercase, drop characters that are not letters, digits, spaces or `-`, spaces → `-`.

- [ ] **Step 1: Write the failing tests** in `scripts/test_check_english.py` (load the module with `importlib` like `scripts/test_check_site.py`):

```python
def test_has_vietnamese(self):
    self.assertTrue(has_vietnamese("Bản nháp"))
    self.assertFalse(has_vietnamese("Draft, §3 ⌘⇧L"))

def test_scan_allows_literals_in_crates(self):
    self.assertFalse(scan_line("crates/x/src/a.rs", 'assert_eq!(f("chào"), "ệ");'))
    self.assertTrue(scan_line("crates/x/src/a.rs", "// Nguồn symbol"))

def test_scan_flags_backticked_vietnamese_in_docs(self):
    self.assertTrue(scan_line("docs/a.md", "Gõ `chào` rồi Enter"))
    self.assertTrue(scan_line("docs/a.md", "see `chào`"))

def test_heading_numbers_kept_for_section_and_alnum_forms(self):
    s = signature("# Tiêu đề\n## 3.1. Một\n### §7.6 Hai\n## V2a Ba\n")
    self.assertEqual(s["headings"], [(1, None), (2, "3.1."), (3, "§7.6"), (2, "V2a")])

def test_structure_detects_table_column_change(self):
    a = signature("| a | `x|y` |\n|---|---|\n")
    b = signature("| a | `x|y` | c |\n|---|---|\n")
    self.assertNotEqual(a["table_rows"], b["table_rows"])
    self.assertEqual(a["table_rows"], [3, 3])

def test_structure_reports_broken_anchor(self):
    s = signature("## Phần mới\nxem [x](#bổ-sung)\n")
    self.assertEqual(s["broken_anchors"], ["#bổ-sung"])
    ok = signature("## Bổ sung\nxem [x](#bổ-sung)\n")
    self.assertEqual(ok["broken_anchors"], [])

def test_fences_and_inline_code_counted(self):
    s = signature("```rust\nlet a = `b`;\n```\nText `c` and `d`.\n")
    self.assertEqual((s["fences"], s["inline_code"]), (2, 2))
```

- [ ] **Step 2: Run** `python3 -m unittest scripts/test_check_english.py -v` — expected: FAIL (module file missing).
- [ ] **Step 3: Implement** `scripts/check-english.py` with the interfaces above (standard library only; read files at a revision with `git show REV:path`; a file absent at `REV` is skipped with a note).
- [ ] **Step 4: Run** the tests again — expected: all PASS. Then run `python3 scripts/check-english.py scan` — expected: exit 1, flagged lines only in `docs/**`, `CLAUDE.md`, `scripts/gen-seti.py`, `crates/lucerna-workbench/src/symbols/source.rs`, `crates/lucerna-theme/tests/lucerna_theme.rs` (about 12,700 lines). Any flagged line elsewhere in `crates/**` means the literal rule is wrong: fix the rule, not the data.
- [ ] **Step 5: Add** the CI step to `.github/workflows/ci.yml`, job `check`, after checkout: `name: English check tests`, `run: python3 -m unittest scripts/test_check_english.py`. Do *not* add a `scan` step yet (it fails until Task 17).
- [ ] **Step 6: Commit**

```bash
git add scripts/check-english.py scripts/test_check_english.py .github/workflows/ci.yml
git commit -m "chore: add check-english script" -- scripts/check-english.py scripts/test_check_english.py .github/workflows/ci.yml
```

---

### Translation tasks (Tasks 2–16)

Every translation task has the same steps; only the file list differs. They depend on Task 1 only and are independent of each other, so they can run in parallel (different files). Record `BASE=$(git rev-parse HEAD)` before Step 2.

- [ ] **Step 1:** `git status --short -- <files>`; skip and report any file with uncommitted changes from someone else.
- [ ] **Step 2:** Translate each file in place following Global Constraints. Read the whole file first; translate section by section with Edit/Write.
- [ ] **Step 3:** `python3 scripts/check-english.py scan <files>` — expected: no output, exit 0.
- [ ] **Step 4:** `python3 scripts/check-english.py structure --base $BASE <files>` — expected: no output, exit 0. A heading-number or code-count difference means something was translated that shouldn't be, or a line was dropped: fix the translation.
- [ ] **Step 5:** Commit: `git add <files>` then `git commit -m "docs: translate <batch> to English" -- <files>`.

| Task | Batch name | Files |
|---|---|---|
| 2 | core docs | `docs/design-principles.md`, `docs/roadmap.md`, `docs/benchmarks.md`, `docs/research/file-icons-screenshots/README.md`, `docs/research/lucerna-theme-screenshots/README.md`, `docs/research/2026-11-09-app-bundle.md`, `docs/research/2026-11-09-extensions.md`, `docs/research/2026-11-09-vim-mode.md` |
| 3 | research LSP and editing | `docs/research/2026-11-10-lsp.md`, `docs/research/2026-11-10-project-search-ui.md`, `docs/research/2026-11-11-editing.md`, `docs/research/2026-11-11-lsp-server-table.md` |
| 4 | research 11-12 to 11-15 | `docs/research/2026-11-12-welcome.md`, `docs/research/2026-11-13-explorer-context-menu.md`, `docs/research/2026-11-13-git.md`, `docs/research/2026-11-13-markdown-preview.md`, `docs/research/2026-11-13-terminal.md`, `docs/research/2026-11-14-default-theme.md`, `docs/research/2026-11-14-scrollbar-mouse.md`, `docs/research/2026-11-15-file-icons.md`, `docs/research/2026-11-15-git-branch-switch.md` |
| 5 | research 11-15 to 11-18, review | `docs/research/2026-11-15-multi-window.md`, `docs/research/2026-11-15-onboarding.md`, `docs/research/2026-11-16-hot-exit.md`, `docs/research/2026-11-18-image-preview.md`, `docs/superpowers/reviews/2026-11-15-release-flow-review.md` |
| 6 | specs phase 1–2 | `docs/superpowers/specs/2026-11-08-lucerna-phase1-design.md`, `…/2026-11-09-landing-page-design.md`, `…/2026-11-09-logo-design.md`, `…/2026-11-09-lucerna-phase2-extensions-design.md` |
| 7 | specs phase 3, vim navigation | `…/specs/2026-11-10-lucerna-phase3-lsp-design.md`, `…/2026-11-10-project-search-tab-design.md`, `…/2026-11-10-vim-navigation-design.md` |
| 8 | specs 11-11, 11-12 | `…/specs/2026-11-11-lsp-server-table-design.md`, `…/2026-11-11-lucerna-phase4-editing-design.md`, `…/2026-11-12-welcome-design.md` |
| 9 | specs 11-13 (a) | `…/specs/2026-11-13-explorer-context-menu-design.md`, `…/2026-11-13-explorer-own-save-decoration-design.md`, `…/2026-11-13-git-view-design.md`, `…/2026-11-13-markdown-preview-design.md` |
| 10 | specs 11-13 (b) | `…/specs/2026-11-13-single-file-open-design.md`, `…/2026-11-13-terminal-design.md`, `…/2026-11-13-vim-editing-design.md`, `…/specs/terminal-ledger.md` |
| 11 | specs 11-14, 11-15 | `…/specs/2026-11-14-editor-benchmark-design.md`, `…/2026-11-14-lucerna-theme-design.md`, `…/2026-11-14-scrollbar-mouse-design.md`, `…/2026-11-15-explorer-file-ops-design.md`, `…/2026-11-15-file-icons-design.md`, `…/2026-11-15-git-branch-switch-design.md`, `…/2026-11-15-multi-window-design.md` |
| 12 | specs 11-15 to 11-17 | `…/specs/2026-11-15-onboarding-design.md`, `…/2026-11-15-release-flow-review-design.md`, `…/2026-11-15-restore-last-workspace-design.md`, `…/2026-11-16-dev-bundle-id-design.md`, `…/2026-11-16-hot-exit-design.md`, `…/2026-11-16-markdown-preview-follows-rename-design.md`, `…/2026-11-16-scan-depth-design.md`, `…/2026-11-17-find-replace-design.md`, `…/2026-11-17-font-zoom-design.md`, `…/2026-11-17-lsp-outermost-root-design.md`, `…/2026-11-17-multi-cursor-design.md`, `…/2026-11-17-p3-fixes-design.md`, `…/2026-11-17-untitled-save-as-design.md` |
| 13 | specs 11-18 | `…/specs/2026-11-18-image-preview-design.md`, `…/2026-11-18-project-replace-design.md`, `…/2026-11-18-reopen-close-others-design.md` |
| 14 | plans (a) | `docs/superpowers/plans/2026-11-12-welcome.md`, `…/2026-11-13-markdown-preview.md`, `…/2026-11-13-open-folder-in-place.md`, `…/2026-11-13-single-file-open.md`, `…/2026-11-14-editor-benchmark.md`, `…/2026-11-14-lucerna-theme.md`, `…/2026-11-14-scrollbar-mouse.md`, `…/2026-11-15-file-icons.md`, `…/2026-11-15-multi-window.md` |
| 15 | plans (b) | `…/plans/2026-11-15-onboarding.md`, `…/2026-11-15-release-flow-review.md`, `…/2026-11-16-hot-exit.md`, `…/2026-11-16-p2-fixes.md`, `…/2026-11-17-find-replace.md`, `…/2026-11-17-font-zoom.md`, `…/2026-11-17-multi-cursor.md` |
| 16 | plans (c) and the mostly-English plans | `…/plans/2026-11-17-p3-cli.md`, `…/2026-11-17-p3-explorer.md`, `…/2026-11-17-p3-quick-open.md`, `…/2026-11-17-p3-save.md`, `…/2026-11-17-untitled-save-as.md`, `…/2026-11-17-vim-editing-v2b.md`, and these with only 1–6 Vietnamese lines: `…/2026-11-08-lucerna-phase1.md`, `…/2026-11-09-logo.md`, `…/2026-11-09-lucerna-phase2-extensions.md`, `…/2026-11-10-lucerna-phase3-lsp.md`, `…/2026-11-10-vim-navigation.md`, `…/2026-11-11-lsp-server-table.md`, `…/2026-11-11-lucerna-phase4-1-editing-core.md`, `…/2026-11-12-lucerna-phase4-2-smart-typing.md`, `…/2026-11-12-phase4-4-conflict.md`, `…/2026-11-13-terminal.md`, `…/2026-11-13-vim-editing-v2a.md`, `…/2026-11-14-git-5-1-core.md`, `…/2026-11-15-explorer-file-ops.md`, `…/2026-11-15-git-branch-switch.md`, `…/2026-11-16-dev-bundle-id.md`, `…/2026-11-18-project-replace.md`, `…/2026-11-18-reopen-close-others.md` |

`…/` stands for the same folder as the first path in its cell. In the mostly-English plans of Task 16, the Vietnamese lines are often test data inside code blocks (e.g. `"xin chào"` in a Rust snippet): keep those, and report them so Task 17 can add them to the scan's allow-list if they remain.

---

### Task 17: Code comments, the rule, and the full scan

Runs after Tasks 2–16 are committed (it quotes translated headings).

**Files:**
- Modify: `crates/lucerna-workbench/src/symbols/source.rs:1` — replace `"Nguồn symbol"` with the translated title of §7.6 in `docs/superpowers/specs/2026-11-10-lucerna-phase3-lsp-design.md` (confirm §7.6 is that spec's "Nguồn symbol" section first; grep the specs if not).
- Modify: `crates/lucerna-theme/tests/lucerna_theme.rs:438` — replace `§ Bảng màu` with the translated heading in `docs/superpowers/specs/2026-11-14-lucerna-theme-design.md`.
- Modify: `scripts/gen-seti.py:26` — replace `"Dữ liệu Seti"` with the translated heading in `docs/superpowers/specs/2026-11-15-file-icons-design.md`.
- Modify: `CLAUDE.md:3` — replace the sentence "Talk to the user in Vietnamese, but every string the app shows … is English only; Vietnamese appears in code only as test data" with: "Talk to the user in Vietnamese. Everything written to the repository is English: code, comments, docs, specs, plans, research, commit messages and PR text, including every string the app shows. Vietnamese appears in the repository only as test data in code (see `docs/design-principles.md` §3)."
- Modify: `docs/design-principles.md` §3 (already English after Task 2) — the "English only in the app" bullet becomes "English only in the repository": same content as the CLAUDE.md sentence, dropping the old allowance that internal specs and docs may be Vietnamese; keep the part that UI text follows VS Code naming.
- Modify: `docs/superpowers/specs/2026-11-21-english-only-repo-design.md` — status line to `Done`.
- Modify: `.github/workflows/ci.yml` — job `check`, right after the Task 1 step: `name: English only`, `run: python3 scripts/check-english.py scan`.
- Memory (outside the repo): `/Users/duykhanh/.claude/projects/-Users-duykhanh-Projects-nosync-lucerna/memory/english-only-repo.md` (type `feedback`: chat in Vietnamese, write everything in the repo in English; why: open source) and its line in `MEMORY.md`.

- [ ] **Step 1:** Make the three code/script edits; `cargo check -p lucerna-workbench -p lucerna-theme --tests` (run `scripts/prune-target.sh` first; don't run while another session's cargo runs: `pgrep -x cargo`) — expected: finishes without errors. `python3 scripts/gen-seti.py --help` — expected: prints usage.
- [ ] **Step 2:** Edit `CLAUDE.md` and design-principles §3; update the spec status.
- [ ] **Step 3:** `python3 scripts/check-english.py scan` — expected: exit 0. If it flags Vietnamese test data inside code blocks in plans (reported by Task 16), add an allow rule in `scan_line` for Markdown fenced code blocks in `docs/superpowers/plans/**` with a test in `scripts/test_check_english.py`, rerun the unit tests, and rerun the scan.
- [ ] **Step 4:** Add the CI scan step; `python3 -m unittest scripts/test_check_english.py` — expected: PASS.
- [ ] **Step 5:** Commit:

```bash
git add CLAUDE.md docs/design-principles.md docs/superpowers/specs/2026-11-21-english-only-repo-design.md crates/lucerna-workbench/src/symbols/source.rs crates/lucerna-theme/tests/lucerna_theme.rs scripts/gen-seti.py scripts/check-english.py scripts/test_check_english.py .github/workflows/ci.yml
git commit -m "docs: English-only repository rule" -- CLAUDE.md docs/design-principles.md docs/superpowers/specs/2026-11-21-english-only-repo-design.md crates/lucerna-workbench/src/symbols/source.rs crates/lucerna-theme/tests/lucerna_theme.rs scripts/gen-seti.py scripts/check-english.py scripts/test_check_english.py .github/workflows/ci.yml
```

- [ ] **Step 6:** Write the memory file and its `MEMORY.md` line.

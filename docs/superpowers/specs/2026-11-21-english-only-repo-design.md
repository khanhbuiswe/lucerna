# English-only repository

- **Date:** 2026-11-21
- **Status:** Draft, awaiting approval

## Why

Lucerna is open source. Contributors read the specs, plans and research
the code cites (`spec §7.2` appears in 132 source files), so everything in
the repository must be in English. Vietnamese is only for chatting with the
maintainer.

## Goals

- Translate every Vietnamese Markdown file in the repository to English:
  41 specs, 39 plans, 22 research notes, 1 review, `docs/benchmarks.md`,
  `docs/design-principles.md`, `docs/roadmap.md` (about 12,700 lines).
- Fix the three non-test places in code that use Vietnamese:
  `crates/lucerna-workbench/src/symbols/source.rs` (cites the heading
  "Nguồn symbol"), `crates/lucerna-theme/tests/lucerna_theme.rs` (cites
  "§ Bảng màu"), `scripts/gen-seti.py` ("Dữ liệu Seti").
- Write the rule down so later sessions follow it: `CLAUDE.md`,
  `docs/design-principles.md` §3, and the maintainer's Claude memory.

## Non-goals

- Vietnamese test data in code (`"xin chào"`, `"ệ"`, Telex IME cases)
  stays: it tests Unicode and Vietnamese input. §3 keeps allowing it.
- Git history and old commit messages are not rewritten (none of the last
  200 subjects contain Vietnamese anyway).
- File names do not change; they are already English.

## Anti-goals

- A translation that changes meaning, drops detail, or "improves" the
  content. This is a translation, not an edit.
- Renumbered sections. Code cites spec sections by number, so every
  heading keeps its number and order.

## Approach

Considered:

1. **Parallel subagents, one batch of files each (chosen).** Each agent
   translates its files in place, keeping structure. Fast, and each file
   is independent.
2. **Translate everything in this one session.** Same result, but about
   350k words through one context is slow and gets summarized mid-way.
3. **Machine translation script.** Cheap, but loses the repo's plain
   technical style and mangles tables, code spans and links.

## Translation rules (for every agent)

- Keep all Markdown structure: heading levels and numbers, tables, lists,
  code blocks, inline code, links, file paths, commit hashes, dates.
- Never translate content inside code blocks or inline code, except
  Vietnamese comments inside code blocks, which are translated.
- Text the app shows is already English in the specs; keep it verbatim.
- Same-file anchor links (`#bổ-sung-...`) are updated to the slug of the
  translated heading. Links into other files by heading text are checked
  the same way.
- Plain, concise technical English, matching the existing English docs
  (`README.md`, `docs/benchmarks.md`). US spelling.
- Status lines translate literally ("Bản nháp chờ duyệt" → "Draft,
  awaiting approval").

## Verification

- A scan of all tracked text files finds Vietnamese only in test data
  inside `crates/**` string literals (allow-list reviewed by hand).
- For every translated file, the list of heading numbers and the count of
  code fences, table rows and links is identical before and after.
- `cargo test` is not needed for doc-only changes; the two code files
  touched are comment-only, checked with `cargo check` on their crates.

## Rule text

`CLAUDE.md` and design-principles §3 say: talk to the maintainer in
Vietnamese; everything written to the repository (code, comments, docs,
specs, plans, research, commit messages, PR text) is English. Vietnamese
in code is allowed only as test data.

## Left to the builder

- How files are grouped into batches, and committing per batch (stage by
  explicit path; other sessions share this checkout).

# Lucerna — V2: Vim editing

- **Date:** 2026-11-13
- **Status:** V2a and V2b implemented
- **Roadmap:** item V2 in §1.3 of `2026-11-08-lucerna-phase1-design.md`
- **Based on:**
  - V1 `2026-11-10-vim-navigation-design.md`
  - Phase 4 `2026-11-11-lucerna-phase4-editing-design.md`, especially §13 "Contract for V2"
- **Research:** `docs/research/2026-11-09-vim-mode.md`, section 6. Sources are Zed `crates/vim` at `b5cd310` (learn the design only) and VSCodeVim (MIT).

## 1. Goals

### 1.1 Outcome

People used to Vim can edit code in Lucerna with Vim grammar, without turning Vim off or reaching for the mouse. This grammar includes:
- operators with motions and counts;
- registers;
- repeat with `.`;
- undo;
- text objects;
- macros;
- basic `:` commands.

Every change still goes through the Phase 4 editing model. Because of this, undo, merging changes when an agent edits the file, the dirty marker on the tab and LSP all work the same as with normal typing.

### 1.2 Success criteria

1. **Basic editing commands.**
   - `dd`, `3dd`, `dw`, `d$`, `ciw` with typed text, `yy`, `p`, `P`, `x`, `r`, `J`, `>>` and `gUiw` give the same results as VSCodeVim on the test cases in §11.1.
   - Each command, including `ciw` + typed text + Esc, is **one** undo step. `u` and ⌃R put the cursor back where the command started.
2. **Repeat with `.`.** `.` repeats the last change, including text typed through Telex. `3.` replaces the original count.
3. **Registers and clipboard.**
   - `"ayy` then `"ap` uses register `a`.
   - `y` writes the system clipboard, so other apps can ⌘V it.
   - `dd` does not overwrite the clipboard, but `p` right after it still pastes the line just deleted.
   - Copy a passage from the browser then type `p` and that passage is pasted.
4. **Macros.** `qa…q` then `5@a` runs the macro 5 times as one undo step. The status bar shows `recording @a` while recording.
5. **The `:` command line.**
   - `:` opens Quick Open in Vim command mode.
   - `:w`, `:q`, `:wq`, `:%s/foo/bar/g` and `:12` run correctly.
   - `:` opened from Visual mode pre-fills `'<,'>`.
6. **An agent edits the file while Vim is in use.** If an agent edits the file while the user is in the middle of a Vim operator, Phase 4.4 still merges the changes or reports a conflict as with normal typing.
7. **Performance.** A Vim command finishes in under 1 ms on a 100k-line file, except `:%s` and search commands on large files (§10).

### 1.3 What was decided during brainstorming

| # | Decision | Reason |
|---|---|---|
| B1 | **The clipboard only receives yanks.** `vim.useSystemClipboard` gains the value `"onYank"` and makes it the default: `y` writes the clipboard (V1 already did this), while `d`, `c`, `x` do not. The unnamed register reads from the clipboard if the clipboard is newer than the last yank (like Zed's `on_yank`). `true` and `false` keep VSCodeVim's meaning. | The user chose it. Copying from the browser then `x` on one character does not lose the clipboard. Differs from VSCodeVim (default `false`) here. |
| B2 | **A pure core in `lucerna-vim`.** This crate computes the operator's range and kind, then builds the `EditPlan` with `lucerna-editor` functions. | Matches the V2 contract (Phase 4 spec §13.1). All tests run without GPUI. |
| B3 | **`.` uses one change record** containing the Normal keys of the command, register, count and **the typing events in Insert** (§3.3). On replay, the typing events go back through the editor's typing path. | The IME has already committed so the text is the final text; auto-close and overtype run again identically because of the same config. |
| B4 | **Macros record keys into a register**, as text with Vim notation (`<Esc>`, `<CR>`, `<C-r>`). Text inserted in Insert is recorded as text. | Like Vim and VSCodeVim: `"ap` can paste a macro, `@x` can run text in a register. Zed separates macros from registers, so it cannot do these two things. |
| B5 | **`:` uses Quick Open**, as Zed uses the command palette. There is no command line in the status bar. | `design-principles.md` §1.1: UI follows Zed. Quick Open already has the `:` prefix (go to line). |
| B6 | **Delivered in two steps:** V2a and V2b (§12). Visual block with operators is left until after Phase 4.3 (multi-cursor). | The scope is large. Visual block needs multiple selections (contract §13.5). |

### 1.4 Not in V2

- Operators in Visual Block (`<C-v>` then `d`, `c`, `I`, `A`…). Waiting for Phase 4.3 to have multi-cursor. For now `<C-v>` only selects and `y` as in V1.
- The `=` and `gq` operators: waiting for Phase 4.6 to have format. The `g?` operator.
- Surround (`ys`, `cs`, `ds`), `gc` (uses the editor's ⌘/), `gR`, `ReplaceWithRegister`, EasyMotion- or Sneak-style plugins.
- Replace mode `R`, line undo `U`, undo tree (`g-`, `:earlier`).
- Text objects `ia`/`aa` (argument), `ii`/`ai` (indent block), `if`/`af`/`ic`/`ac` (function and class by tree-sitter).
- `:` commands outside the list in §8. `:s` with the `c` flag (confirm each). Range `/pat/`. `:` command history. `@:`.
- Key remapping (`vim.normalModeKeyBindings`…), `<leader>`.
- Saving registers or macros across sessions.
- `⌃A` and `⌃X` (increment/decrement numbers).

## 2. References

Details in `docs/research/2026-11-09-vim-mode.md` §6. Which side Lucerna picks when Zed and VSCodeVim differ:

| Point | Zed | VSCodeVim | Lucerna |
|---|---|---|---|
| Default clipboard | `always` | `false` | `"onYank"` (B1) |
| `N.` | replaces the original count | repeats the whole command N times | **replaces the count** like Vim and Zed. VSCodeVim repeating N times deviates from Vim, has no test, and gives odd results (`d2w` then `3.` deletes 6 words) |
| `c` writes `"1` and `"-` | yes | no | yes, like Vim and Zed |
| Rotation of `"1`–`"9` when a named register was specified | does not shift | still shifts | does not shift, like Vim |
| `i(` when the cursor is not inside any pair | does nothing | looks for the next pair | looks ahead (behavior follows VS Code) |
| Count with brackets (`c2i(`) | ignored | goes out past nested pairs | goes out past nested pairs |
| Macros | separate from registers | live in registers | live in registers (B4) |
| `:` command line | command palette | status bar | Quick Open (B5) |
| `vim.joinspaces` | no setting, inserts 1 space | `true` | `true` (follows VS Code) |
| Undo | editor transaction | implemented by diff | Phase 4 transaction (§5) |

License: do not copy code from Zed's `crates/vim`. Test cases adapted from VSCodeVim cite the source in comments and in `THIRD_PARTY_NOTICES.md` (the "VSCodeVim test cases" section already exists).

### Helix (a third view, added when writing the V2b plan)

Read `helix-term/src/commands.rs` (`record_macro`, `replay_macro`), `commands/typed.rs` and `helix-view/src/input.rs` (`parse_macro`) on the `master` branch, along with the docs `keymap.html` and `commands.html`. Learn the design only (MPL-2.0), do not copy code.

- **Macros:** `Q` toggles recording, `q` runs; the default register is `@`, choose another register with `"x`. Keys are stored in the register as **key-notation text** (`<esc>`, `<ret>`…), the last key that stops recording is dropped, so they can be pasted and edited like in Vim (matches B4). A count replays the whole key sequence `count` times.
- **Replay** feeds each key through the whole compositor, so the `:` and `/` prompts inside a macro run exactly as when typed, without needing to know in advance which key belongs to which mode. Lucerna does the same: replaying a macro is replaying keys, and `lucerna-vim` itself collects the text after `/` or `:` while running a macro (plan V2b).
- **Recursion guard:** Helix refuses to run a register that is already running (reports an error). Vim and VSCodeVim allow recursive macros (`qa…@aq`); Lucerna follows Vim but limits the nesting depth.
- **Undo:** Helix does not group a whole macro run into one undo step; Lucerna keeps the one-step decision (§6.4).
- **`:` command line:** commands have long names and a fixed alias list (`write`/`w`, `write-quit`/`wq`, `x`/`xit` saves if there are changes then closes, `quit-all`/`qa`, `write-all`/`wa`), no Vim-style prefix abbreviation, **no ex-style range** and no `:s` (substitution uses selection and `s`). Some standalone ones are read as `:goto`, like how Lucerna reads `:12`.
- **Highlight on yank:** Helix does not have it.
- **Conclusion:** no decision of the spec changes. Behavior still follows VSCodeVim, UI follows Zed.

## 3. Architecture

```
lucerna-editor   + case.rs: change case over ranges → EditPlan
                 + edit_ranges(): replace a list of ranges with text → EditPlan (shared helper)
lucerna-vim      gains a dependency on lucerna-editor (pure crate, no gpui)
                 + operator.rs   Operator, range + kind (Charwise / Linewise / Blockwise, inclusive / exclusive)
                 + object.rs     text objects
                 + edit.rs       operator → EditPlan (d c y > < gu gU g~ ~ J r x p P)
                 + register.rs   Registers, write and read rules, clipboard
                 + repeat.rs     LastChange (for .) and the replayer
                 + macros.rs     record and run macros, key notation <Esc>
                 + ex.rs         : command parser and running :s :d :y (pure)
lucerna-workbench
                 vim.rs          applies Action::Edit via CodeView::transact_group; undo; clipboard;
                                 records inserted text; Quick Open in Vim command mode
                 code_view       + transact_group; CodeViewEvent::Typed in an Insert session
                 quick_open      + QuickOpenQuery::VimCommand
```

### 3.1 `lucerna-vim` builds `EditPlan`

- `ViewInfo` gains `fn edit_ctx(&self) -> Option<lucerna_editor::Ctx<'_>>`, returning `None` when the view is read-only.
- `Action` gains these variants:
  - `Edit(EditPlan)`;
  - `Type(Typed)`, only used on replay (§3.3);
  - `Undo(u32)` and `Redo(u32)`;
  - `WriteClipboard { text, linewise }`;
  - `RunEx(ExCommand)`, for things the workbench does: save, close tab, turn off highlight;
  - `OpenCommandLine { prefill }`;
  - `Message(String)`, for example `3 substitutions on 2 lines`.
- `lucerna-vim` tests build a `Ctx` with the default `EditConfig`, thanks to the `test-util` feature of `lucerna-editor`.
- **One key, one `feed`.** Each `EditPlan` is computed against the text before editing, so the next key must see the changed text. That is why `lucerna-vim` does not run the replay loop itself.
  - `Vim` returns `Outcome::Replay(Vec<ReplayStep>)` on `.`, `@a` or `@@`. `ReplayStep` is `Key(Key)`, `Type(Typed)`, `Search { pattern, backward }` or `Ex(String)`.
  - **The workbench runs the loop** (`*(my call)*`): it feeds each step into `feed` (or into the typing path, search, `:`), applies that step's actions, and only then moves to the next step. The whole thing sits in **one** `transact_group`.
  - **Search during replay always runs synchronously**, even for files over 1 MB. This way a macro is not cut off by a background-thread search.
  - **Failed motion:** each `Outcome` says whether the motion failed, and the workbench stops the replay there (§6.4).

### 3.2 One command is one undo step

- **Grouping.** The workbench applies every `Action::Edit` produced from **one** `feed` in **one** `CodeView::transact_group(|v, cx| …)`.
  - `transact_group` is built on the existing `suppress_stops` flag: push a stop before, hold off pushing stops while running, then push a stop after finishing.
  - This is the function that contract §13.2 of the Phase 4 spec promised.
- **Commands ending in Insert** (`c…`, `s`, `S`, `C`, `o`, `O`, `i` with a count…): the group stays open until Esc, the way an Insert session already works.
- **Macros and `.`:** the whole `@a` or `.` run is one group, even if it contains many commands.

### 3.3 Following inserted text in Insert

- **Typing events.** In an Insert session opened by Vim, CodeView emits `CodeViewEvent::Typed(Typed)` for each **typing action**, not each buffer change. `Typed` has four forms:

  | Form | Recorded when |
  |---|---|
  | `Text(String)` | typing text, or an IME commit (the final text) |
  | `Newline` | Enter |
  | `DeleteLeft` | ⌫ |
  | `Completion { delete_left, delete_right, text }` | accepting a completion item; `text` is the text actually inserted, snippet expanded |

- **Recording.** `VimLayer` forwards these events to `Vim::record_typed(…)`, and the record for `.` and macros (§6) keeps them as they are.
- **Replay** uses the editor's **exact typing path**:
  - `Text` goes through `commands::insert`, so auto-close and overtype run again;
  - `Newline` goes through `commands::newline`;
  - `DeleteLeft` goes through `commands::delete_left`;
  - `Completion` deletes and inserts the exact number of characters around the cursor.

  Example: `ci(` then typing `foo(` and `bar` is recorded as `Text("foo(")`, `Text("bar")`. Replay produces `foo(bar)`, same as the first time, because auto-close also runs again.
- Moving the cursor with the arrow keys in Insert closes the undo group and starts a new record, like VSCodeVim (`<Left>` creates an undo point).

### 3.4 Marks follow every edit

V1 only shifted marks when the file was reloaded. V2 shifts marks by the `Patch` of **every** edit, including edits from another view of the same file, using `Patch::map_offset`. Reason: `dd` above a mark must pull that mark up.

## 4. Operators and motions

### 4.1 Grammar

`["x] [count] operator [count] (motion | text-object)`, or a doubled operator, or a single command.

- **Count:** the counts before and after the operator multiply (`2d3w` = 6 words), like Vim and Zed.
- **Operators:**

  | Key | Effect |
  |---|---|
  | `d` | delete |
  | `c` | delete then enter Insert |
  | `y` | yank |
  | `>` / `<` | indent / outdent, using the editor's `lines::indent_lines` and `outdent_lines` |
  | `gu` / `gU` / `g~` | lowercase / uppercase / toggle case |

- **Doubled** is the current line plus `count−1` lines below: `dd`, `cc`, `yy`, `>>`, `<<`, `guu`, `gugu`, `gUU`, `gUgU`, `g~~`, `g~g~`.
- **`v`, `V` placed between the operator and the motion** force a kind change, like Zed:

  | Original motion | After `v` | After `V` |
  |---|---|---|
  | linewise | exclusive | — |
  | exclusive | inclusive | linewise |
  | inclusive | exclusive | linewise |

- **Esc or an invalid key** midway cancels the operator.
- A failed motion (for example `f` finds no character) cancels, editing nothing.

### 4.2 Motion kinds (following Vim and Zed)

| Kind | Motions |
|---|---|
| Linewise | `j k gg G H M L`, `'x`, `N%`, `+ - <CR>` |
| Inclusive | `$ e E ge gE f t %` |
| Exclusive | `w W b B h l 0 ^ { } F T n N * #`, `` `x ``, `/` and `?` |

**Operators with `/` and `?`** (`d/foo<CR>`, `c?bar<CR>`): the operator is held pending while the find widget is open (V1's `VimSearch` mechanism). Enter completes the operator with the match found. Esc cancels both.

Vim's rules:
- **Exclusive rule:** an end point at column 0 of the next line moves back to the end of the previous line and the motion becomes inclusive. If the start point is then before the first non-blank character, the motion becomes linewise.
- **`dw`:** does not delete across the newline character. `dw` on an empty line deletes that line; `cw` on an empty line does not.
- **`cw` / `cW` on a non-blank character** is `ce` / `cE`. On whitespace, `cw` only replaces the whitespace up to the start of the next word. `vim.changeWordIncludesWhitespace=true` turns this rule off.
- **`cc`, `S`, `c` with a linewise motion:** keep the indentation and do not delete the final newline character.
- **Linewise delete on the last line of the file** takes the preceding newline character too.
- **Cursor position after an operator:**

  | Command | Cursor |
  |---|---|
  | linewise `d` | the first non-blank character of the line that takes the place of the deleted line (per `vim.startofline`) |
  | `y` | the start of the range; if the motion goes forward, stay put |
  | `>`, `<` | the first non-blank character of the first line |
  | `gu`, `gU`, `g~` | the start of the range |

### 4.3 Single commands

| Key | Behavior |
|---|---|
| `x`, `X` | Delete `count` characters after or before the cursor, not crossing lines. Writes `"-`. |
| `s` | `c` over `count` characters (not crossing lines). |
| `S` | Same as `cc`. |
| `C`, `D` | `c$` / `d$`. With a count, goes to the end of the (count−1)th line below. |
| `Y` | Same as `yy`, like Vim. VSCodeVim also does this when not remapped. |
| `r{c}` | Replace `count` characters with `c`. If fewer than `count` characters remain on the line, do nothing. `r<CR>` replaces with a single newline. The cursor is on the last replaced character. |
| `~` | Toggle case of `count` characters then move right, not crossing lines. With `vim.tildeop=true`, `~` becomes an operator like `g~`. |
| `J` | Join `max(count, 2)` lines: strip the next line's indentation and insert one space, unless the previous line is empty or ends in whitespace, or the next line starts with `)`. With `vim.joinspaces=true`, insert two spaces after `.`, `!`, `?`. The cursor is at the join point. |
| `gJ` | Like `J` but does not strip whitespace and does not insert a space. |
| `p`, `P` | Table in §6.3. |
| `gp`, `gP` | Like `p`/`P`, with the cursor placed right after the pasted text. |
| `u`, `⌃R` | Undo or redo `count` steps of the document. The cursor goes to the selection before the group (for undo) or after the group (for redo), then is clamped to Normal. When out of steps, do nothing. |
| `i a I A o O` with a count | `3ifoo<Esc>` inserts `foofoofoo`. `3o` opens three lines with the same text. |
| `.` | §6.1. |

`x`, `X`, `D`, `C`, `s`, `S`, `r`, `J`, `~`, `p` and `P` also set the record for `.`.

## 5. Text objects

They follow an operator, or are used in Visual mode (extending the selection). In the table below, `i…` is inside (inner) and `a…` is the whole surrounding (around).

| Object | Rule |
|---|---|
| `iw` `aw` `iW` `aW` | A word by V1's character classes. `aw` takes the whitespace after; if there is none, takes the whitespace before. A word at the start of a line does not take the indentation. Standing on whitespace: `iw` takes the whole whitespace run, `aw` takes the whitespace plus the next word. With a count it takes several words, even across lines. |
| `i(` `a(` `ib` `ab` `i{` `a{` `iB` `aB` `i[` `a[` `i<` `a<` (also `)` `}` `]` `>`) | The bracket pair enclosing the cursor, across multiple lines and nested. If the cursor is not inside any pair, look for the next pair ahead (VSCodeVim). With a count, go out past nested pairs. When a syntax tree is available, brackets inside strings or comments are skipped (`enclosing_nodes`). Multi-line `i{` (opening bracket at the end of a line, closing bracket at the start of a later line after whitespace): the inside is **the lines in between, linewise** like Vim, so `di{` leaves `{\n}`, `yi{` yanks linewise, `ci{` acts like `cc` (keeps indentation). |
| `i"` `a"` `i'` `a'` `` i` `` `` a` `` | Searches only the current line. Skips quotes preceded by `\`. Determines the pair by counting quotes from the start of the line. If the cursor is before the first pair, takes the next pair. `a"` takes the whitespace after, or before if there is none. If the cursor is after the last pair, it fails. |
| `it` `at` | The HTML, XML or JSX tag enclosing the cursor. With a syntax tree, finds the node whose opening and closing tags have the same name. Without a tree, scans the text for a matching tag pair by name. |
| `ip` `ap` | A paragraph is a block of lines with text, or a block of empty lines. `ap` also takes the empty block after, or before if at the end of the file. Always linewise. |
| `is` `as` | A sentence (ending in `.`, `!` or `?` then whitespace). `as` takes the whitespace after. |

## 6. Registers, `.` and macros

### 6.1 `.` (repeat)

- **The `LastChange` record** contains:
  - the specified register;
  - the count;
  - the command's keys in Normal or Visual;
  - the list of `Insertion` if the command enters Insert;
  - for a Visual command: the shape of the selection (number of lines and columns, or the number of lines for `V`).
- **Commands recorded:** every command that changes text. Yank, motions and undo are not recorded.
- **Count:** `N.` replaces the record's count. With no count, the original count is used.
- **Register:** `"1p` then `.` becomes `"2p` (the numbered register rotation advances), like Vim and VSCodeVim.
- **Visual commands:** on replay, recreate a selection of the same shape from the current cursor.
- **Replay:** returns `Outcome::Replay` (§3.1). The workbench feeds the keys again, then the `Typed` events through the typing path, then Esc. The whole thing is one undo group.
- **Commands with `/` or `?`** (`d/foo<CR>`): the record keeps the typed pattern as `Search { pattern, backward }`, and on replay it searches directly, without opening the find widget.

### 6.2 Registers

- **Register names:**

  | Name | Meaning |
  |---|---|
  | `"` | unnamed |
  | `a`–`z` | named registers; `A`–`Z` append to the corresponding register |
  | `0` | the last yank |
  | `1`–`9` | the delete ring |
  | `-` | the small delete |
  | `_` | black hole |
  | `+`, `*` | system clipboard |
  | `.` | the last inserted text, read-only |
  | `%` | the file path, read-only |
  | `:` | the last `:` command, read-only |
  | `/` | the last search pattern, read-only |

- **Content:** each register holds `{ text, kind: Charwise | Linewise }`. Blockwise is reserved for when Visual block exists.
- **Write rules** (following Vim):
  - **Yank:** writes `"` and `0`. If a named register was specified, writes that register and `"`, not `0`.
  - **Delete or change** (`d c x X s S C D`): writes `"`.
    - Text containing `\n` or a linewise kind rotates the ring `1`–`9` and writes to `1`.
    - Otherwise writes `-`.
    - If a named register was specified, writes that register and `"`, and does not touch the numbered ring or `-`.
  - **Append `A`–`Z`:** the kind becomes linewise if the appended part is linewise. The text is `old + ("\n" if linewise and `old` does not end in `\n`) + new`.
  - **`_`:** writes nothing, not even `"`.
- **Clipboard per `vim.useSystemClipboard`:**

  | Value | Writes the clipboard | Where `"` reads from |
  |---|---|---|
  | `"onYank"` (default) | every yank with no register specified (a yank into a named register such as `"ayy` does not), like Vim's `clipboard=unnamed` | the clipboard if it **has changed since the most recent write of `"`** (yank or delete); otherwise the internal register |
  | `true` | every write to `"` | the clipboard |
  | `false` | only when using `"+` or `"*` | the internal register |

  - **Kind when reading the clipboard:** `Linewise` if Phase 4's `whole_line` metadata (§5, Clipboard) is true, or the text exactly matches the most recent linewise write. Otherwise `Charwise`. This way ⌘C on a line with nothing selected then `p` pastes linewise.
  - **Detecting "changed"** (`*(my call)*`):
    - On every write of `"` (yank or delete), `Registers` stores a trace of the clipboard text at that moment (a hash). For a yank, that is exactly the text just written.
    - When reading `"`, the clipboard is used if the current hash differs from that trace.
  - **Example:** `yy` on line A → copy B in the browser → `dd` on line C → `p` pastes **C**, because at `dd` the trace was B and the clipboard was still B. Whereas `yy` A → copy B → `p` pastes **B**.
  - **The workbench reads the clipboard** before every `feed` that writes or reads `"`. That is operators, `x`, `p`, `P` and `"+`. This is cheap, since it is just a string in GPUI's memory.

### 6.3 Paste

| Kind | `p` | `P` | Cursor |
|---|---|---|---|
| Charwise, one line | after the cursor | at the cursor | the last character of the pasted text |
| Charwise, multiple lines | after the cursor | at the cursor | the start of the pasted text |
| Linewise | a new line below | a new line above | the first non-blank character of the first pasted line |

- A count repeats the text: `3p`.
- `gp` / `gP` place the cursor right after the pasted text. For the linewise kind, that is column 0 of the next line.
- **Visual `p`:** replaces the selection, and the unnamed register receives the old selection. Visual `P` does not overwrite the unnamed register.
- An empty register does nothing.

### 6.4 Macros

- **Record:** `q{a-z}` starts recording, `q` again stops; `q{A-Z}` appends to the old macro.
  - The recorded content is the keys that were `feed` and the `Typed` events in Insert (§3.3), written as text in Vim notation:

    | What is recorded | Written as |
    |---|---|
    | `Text` | the text verbatim |
    | `Newline` | `<CR>` |
    | `DeleteLeft` | `<BS>` |
    | special keys | `<Esc>`, `<Tab>`, `<C-x>`, `<lt>` |
    | `Completion` | the inserted text, along with the number of `<BS>` matching the part deleted on the left; the part deleted on the right is ignored. This is an approximation, stated clearly in the README. |
    | search | `/pat<CR>` or `?pat<CR>`, like Vim |
    | `:` command | `:cmd<CR>`, like Vim |
    | `/`, `?`, `:` cancelled by Esc after an operator, count or `'<,'>` | `d/pat<Esc>`, `:<Esc>`: on rerun, those same things are dropped too |

  - While running a macro, `/`, `?` and `:` do not open the find widget or Quick Open. `lucerna-vim` collects the text typed after them into a separate internal command line; `<CR>` runs it (search with the defaults of a fresh find widget, not taking the flags of `*`), `<Esc>` drops it.
  - A macro is stored in the register as charwise text, so `"ap` can paste it.
- **Run:** `[count]@{a-z}` runs the macro `count` times. `@@` reruns the most recently run macro.
  - Any text in a register can also be run: it is read by the notation above.
  - If a motion fails while running (for example `f` finds nothing, `n` runs out of matches with `wrapscan` off), the whole macro stops, like VSCodeVim.
  - The whole run is one undo group.
  - While running, the numbered register ring does not shift and `.` is not overwritten.
- **Display:** while recording, the mode indicator shows `-- NORMAL -- recording @a` (both Zed and VSCodeVim do this).

## 7. Visual mode with operators

Applies to Visual and Visual Line.

| Key | Behavior |
|---|---|
| `d`, `x` | Delete the selection. |
| `D`, `X` | Delete linewise. |
| `c`, `s` | Delete then enter Insert. |
| `C`, `S`, `R` | Like `c`, but linewise. |
| `y` | Like V1, but now writes registers per §6.2. |
| `Y` | Yank linewise. |
| `>`, `<` | Indent / outdent `count` levels. |
| `~`, `u`, `U` | Toggle case / lowercase / uppercase. |
| `r{c}` | Replace every character in the selection (except newlines) with `c`. |
| `J`, `gJ` | Join the lines of the selection. |
| `p`, `P` | §6.3. |

- After each operator, the cursor goes to the start of the selection and Vim returns to Normal (Insert for `c`).
- Marks `<` and `>` are updated.
- `:` from Visual opens the command line with `'<,'>` pre-filled.

## 8. The `:` command line (step V2b)

### 8.1 UI (following Zed)

- **The corresponding screen in Zed:** `:` opens the command palette. That is a modal at the top center of the window, one input box and a result list below (`command_palette` + `command_interceptor` of the `vim` crate).
- **Lucerna:** `:` in Normal or Visual opens **Quick Open** (also a modal at the top center, already following Zed) with the text `:` already in the input box.
- **The list below:**
  - A single line describing the command that will run, for example `:%s/foo/bar/g — Replace in all lines`, `:w — Save`, `:12 — Go to line 12`.
  - An invalid command shows a dimmed error line, for example `Not an editor command: foo`.
  - Enter runs the command, Esc closes.
- **Behavior when Vim is off:** `:` in Quick Open is still just go-to-line as today. Only when Vim is on is anything after `:` that is not a number read as a Vim command. A number (`-?N`, optionally with `:` `,` `#` and `-?M`) keeps VS Code's go-to-line meaning: `:-1` is the last line, `:12,5` is line 12, column 5.
- **A command's result** (`3 substitutions on 2 lines`, `E492: Not an editor command`) shows in the status bar, in the mode indicator's place, until the next key.
- **Different from Zed:** there is no suggestion list of many commands, because Lucerna has only a few `:` commands.

### 8.2 Commands

| Command | Effect |
|---|---|
| `:N`, `:$` | Go to line (keeps the existing go-to-line behavior). |
| `:w`, `:write` | `workbench.action.files.save`. |
| `:w!` | Save, ignoring the conflict warning (Phase 4.4's `files.saveConflictResolution`: overwrite). |
| `:wa`, `:wall` | `workbench.action.files.saveAll`. |
| `:q`, `:quit` | `workbench.action.closeActiveEditor`. A dirty tab shows Phase 4's prompt. |
| `:q!` | Revert then close the tab, without asking. |
| `:qa`, `:qall` | Close all tabs (with prompt). |
| `:wq`, `:x`, `:xit` | Save then close the tab. `:x` only saves when the file is dirty. |
| `:[range]d [x] [count]` | Delete lines into a register. |
| `:[range]y [x] [count]` | Yank lines. |
| `:[range]>`, `:[range]<` | Indent / outdent. |
| `:[range]s/pat/rep/[flags] [count]` | Substitute, see below. |
| `:noh`, `:nohlsearch` | Turn off search highlight. |

Abbreviations follow a `[short, full]` table like Zed and VSCodeVim.

**Range:**
- `N`, `.`, `$`, `%`, `'x`, `'<,'>`, `+N`, `-N`, and combinations like `.+3`, `'a,$`.
- A reversed range is flipped.
- `3:` opens with `.,.+2`.

**`:s`:**
- **Delimiter:** any character that is not a letter or digit.
- **Pattern:** written in Rust regex syntax, with the common Vim notations converted: `\(` `\)` `\{` `\}` `\+` `\?` `\|` `\<` `\>`.
- **Empty pattern:** uses the last search pattern. `:s` with nothing after it repeats the last substitution.
- **Replacement:**

  | Notation | Meaning |
  |---|---|
  | `&`, `\0` | the whole match |
  | `\1`–`\9` | groups |
  | `\n` | newline |
  | `\t` | tab |
  | `\\`, `\&` | the character itself |
  | `~` | the previous replacement |
  | `\u` `\U` `\l` `\L` `\e` `\E` | case change |

- **Flags:** `g`, `i`, `I`, `n` (count only). `vim.gdefault=true` inverts the `g` flag.
- **Default range:** the current line.
- **After running:**
  - It is one undo step.
  - The cursor is at column 0 of the last substituted line.
  - The pattern becomes the last search pattern, so `n` works.
  - The status bar reports `N substitutions on M lines`.
- **No match:** `E486: Pattern not found: foo`.

## 9. New settings

| Key | Type | Default | Source |
|---|---|---|---|
| `vim.useSystemClipboard` | bool \| `"onYank"` | `"onYank"` | VSCodeVim (bool, default `false`). Lucerna adds `"onYank"` (B1). |
| `vim.gdefault` | bool | false | VSCodeVim |
| `vim.joinspaces` | bool | true | VSCodeVim |
| `vim.changeWordIncludesWhitespace` | bool | false | VSCodeVim |
| `vim.tildeop` | bool | false | VSCodeVim |
| `vim.highlightedyank.enable` | bool | false | VSCodeVim (step V2b) |
| `vim.highlightedyank.duration` | number (ms) | 200 | VSCodeVim (step V2b) |

- `>` and `<` use `editor.tabSize` and `editor.insertSpaces`, like VSCodeVim. There is no `vim.shiftwidth`.
- The yank highlight color comes from the theme (`editor.findMatchHighlightBackground`), with no `vim.highlightedyank.color`. This deviates from VSCodeVim, because the UI follows Zed's theming.

## 10. Performance and errors

- **Cost of a command:** operators, text objects and registers only read a few lines around the cursor.
  - `ip`/`ap` and `i{` may have to scan far, but each line is checked only once.
  - Brackets use `enclosing_nodes` when a syntax tree exists (O(depth)), and scan text only when there is no tree.
- **`:%s` on a large file:**
  - Files under 1 MB: runs synchronously.
  - Larger files: runs on a background thread over a snapshot. Afterwards applies the result if the revision has not changed; if it has changed, reports `E: file changed, :s cancelled`.
  - Keys typed while waiting do not cancel `:s` (like Vim's typeahead). A save or close command (`:w`, `:wa`, `:q`, `:qa`, `:wq`, `:x`) in that view reports `E: :s still running` and does nothing.
- **Errors:**
  - A panic in `feed` or while building a plan is caught as in V1 (`catch_panic`): nothing is applied, Vim returns to Normal.
  - Read-only view: editing operators do nothing. `y` still works.
  - Undo or redo when out of steps: does nothing.

## 11. Testing

### 11.1 `lucerna-vim`, table tests with cursor markers

- **About 70 cases adapted from VSCodeVim**, list and sources in research note §6. Each case records the original file and test name:

  | VSCodeVim file | Content |
  |---|---|
  | `test/mode/modeNormal.test.ts` | `dw` across lines, `cw`, `c2w`, `de`, `dj`, `d}`, `3dd` at the end of the file, `d2gg`, `D`/`C` with a count, `Nr`, `r<CR>`, `Nx`, `~`, `g~~`, `guw`, `gUae` |
  | `normalModeTests/commands.test.ts` | `J` with a count, joinspaces, `)`, `gJ` |
  | `test/register/register.test.ts` | numbered ring, `"-`, `"0`, `"_`, append `"A` |
  | `test/operator/put.test.ts` | `p` and `gp` with a count |
  | `modeVisualLine.test.ts` | `Vp` |
  | text objects | `cib` looks ahead, `c3i)`, `ci"` standing after, `ca"`, `c2i"`, `daw` on both sides, `d2aw`, `cit`/`cat`, `dip`/`dap`, `das` |
  | `normalModeTests/dot.test.ts` | `.` |
  | `undo.test.ts` | undo |
  | `test/macro.test.ts` | macros |
  | `test/cmd_line/substitute.test.ts`, `delete.test.ts`, `yank.test.ts` | `:` commands |

- **Lucerna's own cases:**
  - `ciw` on `"Xin chào"` and on emoji.
  - `dd` on a one-line file and an empty file.
  - Linewise `p` on the last line without `\n`.
  - Registers through the clipboard with the three values of `vim.useSystemClipboard`.
  - `3.` replaces the count.
  - A macro stops when a motion fails.
  - A macro containing Vietnamese text.
  - `:s` with `\U&`.
  - Range `'a,'b`.
  - A panic while building a plan does not corrupt state.

### 11.2 `lucerna-workbench`, using `TestAppContext`

- **Undo:** `3dd` then `u` restores all three lines in one step. `ciw` + typing (through the input handler, including marked text) + Esc then `u` is also one step.
- **`.` after Insert:** typing with auto-close (`ci(` typing `foo(`), then `.` elsewhere gives the same text.
- **Clipboard:**
  - `y` writes a `ClipboardItem`.
  - `dd` does not write the clipboard.
  - When the clipboard changes from outside, `p` pastes the clipboard.
  - ⌘C on a line (`whole_line` metadata) then `p` pastes linewise.
- **Agent edits the file:** the file changes on disk while an operator is unsaved, then merge or conflict as in Phase 4.4.
- **Marks:** follow a `dd` above.
- **Macros:** `qa…q`, `3@a`, the `recording @a` indicator.
- **The `:` command line:**
  - `:` opens Quick Open; `:w` saves; `:q` on a dirty tab shows the prompt; `:%s` substitutes and reports the count.
  - With Vim off, `:12` still goes to the line.
- **Read-only view** (picker preview): `dd` does nothing.

### 11.3 Manual check

Turn on Vim, then edit a Rust file while Claude Code edits the same file. Type `ciw` with Telex. Use `:` and compare with Zed's command palette.

## 12. Delivery steps

| Step | Content | Depends on |
|---|---|---|
| **V2a: operators** | §3 (except the command-line part); §4; §5; §6.1–§6.3; §7; settings `useSystemClipboard`, `joinspaces`, `changeWordIncludesWhitespace`, `tildeop`; marks following edits; `transact_group`; `CodeViewEvent::Typed`; operators with `/` `?`. | Phase 4.1, 4.2 (done) |
| **V2b: macros and `:`** | §6.4 macros; §8 command line; `vim.gdefault`; yank highlight. | V2a |
| **After 4.3** | Operators in Visual Block; Blockwise registers; `I` and `A` on blocks. | Phase 4.3 |

Each step has its own plan, per-task review and whole-branch review. After each step, update the README, the roadmap table (§1.3 of the Phase 1 spec) and `THIRD_PARTY_NOTICES.md`.

## 13. Risks

| Risk | Mitigation |
|---|---|
| Text inserted through an IME or completion does not match the `.` record | Record the **typing actions** (text after the IME commits, Enter, ⌫, completion), then replay through the exact typing path. Tests with marked text, auto-close, overtype and completion. |
| `transact_group` nesting with the Insert session's group, or with Phase 4's undo stops | A single place opens and closes groups in `VimLayer`. Undo tests for every command ending in Insert. |
| The `"onYank"` clipboard is hard to predict when the user copies in another app | Compare the clipboard text with the last yank, like Zed. Tests for each case. Stated clearly in the README. |
| Vim's exclusive and linewise rules are easy to get wrong | Adapt many test cases from VSCodeVim, plus own cases for the last line without `\n`. |
| `:` key conflict with Quick Open when Vim is off | Only read Vim commands when Vim is on and what follows `:` is not a number. A test with Vim off. |

# Vim mode

Vim is built in and off by default (V1 navigation and V2a editing are done). It takes effect as soon as you save the setting.
The status bar shows the mode, and the cursor becomes a block. Every command is one undo step,
and a command that ends in Insert keeps the step open until `Esc`. Macros and `:` commands are
planned (V2b).

| Group | Keys |
|---|---|
| Motions | `h j k l`, `w b e ge` (and `W B E gE`), `0 ^ $`, `gg G`, `{ }`, `%`, `H M L`, `f F t T` with `;` `,`, counts |
| Search | `/` and `?` (open the find widget), `n N`, `* #` |
| Scroll | `Ctrl+d` `Ctrl+u`, `Ctrl+f` `Ctrl+b`, `zz zt zb` |
| Marks | `m{a-z}`, `` `x ``, `'x`, `` `` `` and `''` |
| Visual | `v`, `V`, `Ctrl+v`, `o`, `Esc` |
| Folds | `za zc zo` |

In Normal and Visual mode the arrow keys act as `h j k l`, and Enter moves to the first
non-blank character of the next line.

**Editing (V2a)**

| Group | Keys |
|---|---|
| Operators | `d c y > < gu gU g~` with a motion or text object and counts (`2d3w` is six words); doubled for lines (`dd cc yy >> << guu gUU g~~`); `v` / `V` after the operator force charwise / linewise |
| Search motions | `d/foo<CR>`, `c?bar<CR>`: the operator waits while the find widget is open; `Esc` cancels both |
| Text objects | `iw aw iW aW`, `i( a( ib ab i{ a{ iB aB i[ a[ i< a<`, quotes (`i" a" i' a'` and backtick), `it at`, `ip ap`, `is as`, with counts; they also extend the selection in Visual mode |
| Single keys | `x X s S C D Y`, `r{c}`, `~`, `J gJ`, `p P gp gP`, counts on `i a I A o O` (`3ifoo<Esc>`) |
| Registers | `"a`-`"z` (`"A`-`"Z` append), `"0` last yank, `"1`-`"9` delete ring, `"-` small delete, `"_` black hole, `"+` `"*` clipboard, read-only `".` `"%` `"/` |
| Repeat | `.` repeats the last change, including text typed in Insert (typing replays through auto-close); one replay is one undo step |
| Undo | `u`, `Ctrl+r`, with counts |
| Visual / Visual Line | `d x D X c s C S R y Y > < ~ u U r{c} J gJ p P` |

Clipboard rule for `"onYank"`: every yank without a register name also goes to the system clipboard
(`"ayy` does not, as with Vim's `clipboard=unnamed`), and `p` pastes the clipboard instead of the
unnamed register only when the clipboard changed since the last write to `"`. So `yy`, copy
something elsewhere, `p` pastes the copied text, but `yy`, copy elsewhere, `dd`, `p` pastes the
deleted line. `"onYank"` is the default here; VSCodeVim defaults `vim.useSystemClipboard` to
`false`.

Known deviations: Visual Block operators come after multi-cursor; macros (`q`, `@`) and `:`
commands are V2b; an arrow key in Insert breaks the `.` record but keeps one undo step.

| Setting | Default | Meaning |
|---|---|---|
| `vim.enabled` | `false` | Turn Vim navigation on. `vim_mode` is read as an alias when this is unset |
| `vim.ignorecase` | `true` | Ignore case for `*` and `#` |
| `vim.wrapscan` | `true` | Searches wrap around the end of the file |
| `vim.startofline` | `true` | `gg`, `G`, `H`, `M`, `L` go to the first non-blank character |
| `vim.scroll` | `0` | Lines for `Ctrl+d` / `Ctrl+u`; `0` means half a page |
| `vim.useSystemClipboard` | `"onYank"` | `"onYank"`: yanks without a register name go to the clipboard and `p` pastes it when it changed since; `true`: every write to `"` goes to the clipboard; `false`: only `"+` / `"*` |
| `vim.joinspaces` | `true` | `J` puts two spaces after `.`, `!` and `?` |
| `vim.changeWordIncludesWhitespace` | `false` | `cw` on a word also changes the whitespace after it, like `dw` |
| `vim.tildeop` | `false` | `~` is an operator like `g~` |
| `editor.cursorSurroundingLines` | `0` | Lines kept around the cursor when it moves |

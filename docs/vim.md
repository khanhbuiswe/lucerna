# Vim mode

Vim is built in and off by default (V1 navigation, V2a editing and V2b macros and `:` commands are done). It takes effect as soon as you save the setting.
The status bar shows the mode, and the cursor becomes a block. Every command is one undo step,
and a command that ends in Insert keeps the step open until `Esc`.

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

**Macros and command line (V2b)**

| Group | Keys |
|---|---|
| Record | `q{a-z}` starts recording into a register, `q{A-Z}` appends to it, `q` stops. The status bar shows `-- NORMAL -- recording @a` |
| Play | `[count]@{reg}` plays a register (`5@a`), `@@` repeats the last one. A whole run, even `5@a`, is one undo step |
| Command line | `:` opens Quick Open as a command line (`:w — Save`, `:12 — Go to line 12`, `:%s/foo/bar/g — Replace in all lines`). From Visual mode it starts with `'<,'>`; a count (`3:`) starts with `.,.+2` |
| Commands | `:w :w! :wa :q :q! :qa :wq :x :N :$ :d :y :> :< :s :noh` |
| Ranges | `N`, `.`, `$`, `%`, `'<,'>`, `'a`, with `+N` and `-N` offsets, joined by `,` |
| `:s` flags | `g` (all matches in a line), `i` (ignore case), `I` (match case), `n` (count only), and a trailing count; `:s` alone repeats the last substitute. An empty pattern (`:s//x/`) reuses the last search's pattern and its case setting, so after `*` or `#` it follows `vim.ignorecase`. `c` is not supported |
| Messages | `E492: Not an editor command: foo`, `E486: Pattern not found: foo`, `N substitutions on M lines`, `E: file changed, :s cancelled`, `E: :s still running` (a save or close while a large `:s` runs) |

`:s` patterns use Rust regex syntax, with Vim's common escapes translated: `\(` `\)` `\{` `\}`
`\+` `\?` `\|` `\<` `\>`. Unescaped `(` `)` `+` `?` `|` `{` keep their regex meaning, unlike
Vim's default "magic" mode. `:s` is case-sensitive by default; the `i` and `I` flags change that,
and `vim.ignorecase` applies only to `*` and `#`. `:s` matches within a line. With
`vim.gdefault` the `g` flag is on by default, and typing `g` turns it off.

`:qa!` is not supported, only `:qa`. Numbers after `:` keep VS Code's go-to-line meaning: `:12`
goes to line 12, `:-1` to the last line, `:12,5` to line 12, column 5. Anything else is a Vim
command line, so `:+2`, `:$` and `:.,.+2` are Vim's.

Known deviations: Visual Block operators come after multi-cursor; an arrow key in Insert breaks
the `.` record and is not recorded in a macro (one undo step is kept); a macro records
`Completion` only approximately (when Phase 4.5 lands).

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
| `vim.gdefault` | `false` | `:s` replaces all matches in a line without the `g` flag |
| `vim.highlightedyank.enable` | `false` | Flash the yanked text |
| `vim.highlightedyank.duration` | `200` | Milliseconds the yank highlight stays |
| `editor.cursorSurroundingLines` | `0` | Lines kept around the cursor when it moves |

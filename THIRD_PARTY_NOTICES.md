# Third-party notices

Lucerna is licensed under MIT OR Apache-2.0. It depends on third-party crates,
whose licenses are restricted by `deny.toml` to: MIT, Apache-2.0,
BSD-2-Clause, BSD-3-Clause, ISC, Zlib, Unicode-3.0, MPL-2.0, and
`Apache-2.0 WITH LLVM-exception` (see Wasmtime and Cranelift below).

## GPUI

The UI framework `gpui` (and `gpui_platform` and its platform crates) comes
from the Zed repository (https://github.com/zed-industries/zed), pinned to a
single commit in the workspace `Cargo.toml`. Zed's `zed-font-kit` fork (https://github.com/zed-industries/font-kit) is also
pulled in transitively for macOS glyph rasterization. Only the Apache-2.0 licensed
GPUI crates are used; no GPL-licensed crate from that repository is a
dependency.

## Tree-sitter grammars

`lucerna-syntax` bundles these grammars. Each crate compiles its grammar's C
sources into the binary, and every highlights query is the one the crate
itself ships (`HIGHLIGHTS_QUERY` or equivalent); no query file is vendored.
All are MIT licensed.

| Crate | Version | License | Upstream |
|---|---|---|---|
| `tree-sitter` (core library) | 0.27.2 | MIT | https://github.com/tree-sitter/tree-sitter |
| `tree-sitter-language` | 0.1.9 | MIT | https://github.com/tree-sitter/tree-sitter |
| `tree-sitter-rust` | 0.24.2 | MIT | https://github.com/tree-sitter/tree-sitter-rust |
| `tree-sitter-typescript` (TypeScript, TSX) | 0.23.2 | MIT | https://github.com/tree-sitter/tree-sitter-typescript |
| `tree-sitter-javascript` | 0.25.0 | MIT | https://github.com/tree-sitter/tree-sitter-javascript |
| `tree-sitter-python` | 0.25.0 | MIT | https://github.com/tree-sitter/tree-sitter-python |
| `tree-sitter-go` | 0.25.0 | MIT | https://github.com/tree-sitter/tree-sitter-go |
| `tree-sitter-json` | 0.24.8 | MIT | https://github.com/tree-sitter/tree-sitter-json |
| `tree-sitter-md` (block + inline) | 0.5.4 | MIT | https://github.com/tree-sitter-grammars/tree-sitter-markdown |
| `tree-sitter-toml-ng` | 0.7.0 | MIT | https://github.com/tree-sitter-grammars/tree-sitter-toml |
| `tree-sitter-yaml` | 0.7.2 | MIT | https://github.com/tree-sitter-grammars/tree-sitter-yaml |
| `tree-sitter-bash` | 0.25.1 | MIT | https://github.com/tree-sitter/tree-sitter-bash |
| `tree-sitter-html` | 0.23.2 | MIT | https://github.com/tree-sitter/tree-sitter-html |
| `tree-sitter-css` | 0.25.0 | MIT | https://github.com/tree-sitter/tree-sitter-css |

## Wasmtime and Cranelift

`lucerna-syntax` runs extension grammars (tree-sitter compiled to WASM) through
`tree-sitter`'s `wasm` feature, which embeds Wasmtime
(https://github.com/bytecodealliance/wasmtime) and its code generator
Cranelift. They and their related crates are licensed `Apache-2.0 WITH
LLVM-exception`, which `deny.toml` allows; no other license change was made.
Building `wasmtime-c-api-impl` needs `cmake` on the build machine.

wasi-sdk (https://github.com/WebAssembly/wasi-sdk, version 34) is not shipped:
Lucerna downloads it on the user's machine the first time an extension grammar
is built, and uses it only to compile the grammar's C sources. Lucerna does not
distribute any extension either; each user builds them locally from the
`zed-industries/extensions` registry.

## Test fixture: `json.wasm`

`crates/lucerna-syntax/tests/fixtures/wasm/json.wasm` is `tree-sitter-json`
0.24.8 (MIT) compiled with wasi-sdk 34.0, used only by tests and benchmarks.
The upstream license text is `crates/lucerna-syntax/tests/fixtures/wasm/LICENSE-tree-sitter-json`;
the build command is recorded in the README next to it.

## Injection queries

The injection queries of the bundled grammars are the ones their crates ship,
never vendored copies: `tree_sitter_md::INJECTION_QUERY_BLOCK` and
`INJECTION_QUERY_INLINE` (`tree-sitter-md`, MIT) for Markdown, and
`tree_sitter_html::INJECTIONS_QUERY` (`tree-sitter-html`, MIT) for HTML.
Injection queries of extension languages are read from the installed
extension's own `languages/<name>/injections.scm`.

## Website fonts

The landing page in `site/` self-hosts these fonts. Each file is the latin
variable `woff2` build from Fontsource. License texts ship next to the files in `site/fonts/`.

| Font | License | Upstream |
|---|---|---|
| Bricolage Grotesque | OFL-1.1 | https://github.com/ateliertriay/bricolage |
| Instrument Sans | OFL-1.1 | https://github.com/Instrument/instrument-sans |
| JetBrains Mono | OFL-1.1 | https://github.com/JetBrains/JetBrainsMono |

## Additional licenses

Licenses outside the base list that were added to `deny.toml` are recorded
here with the crate that required them.


- `notify` (file watching in `lucerna-project`): CC0-1.0, a public-domain
  dedication. Allowed for this crate only through a `[licenses.exceptions]`
  entry in `deny.toml`; the global allow list is unchanged.

## Themes

`lucerna-theme` bundles the "One" theme family (One Dark, One Light) as a data
file: `assets/themes/one/one.json` from the Zed repository
(https://github.com/zed-industries/zed) at revision
`b5cd310b3b54dfc659fcc06453e6358843dfe216`, stored as
`crates/lucerna-theme/assets/one.json`. It is MIT licensed (Copyright (c) 2014
GitHub Inc.); the license text is `crates/lucerna-theme/assets/LICENSE-one`.

## Fonts

`lucerna-workbench` embeds these fonts with `include_bytes!` and registers them
with GPUI at startup. Both are licensed under the SIL Open Font License 1.1;
each license text is copied verbatim into `crates/lucerna-workbench/assets/fonts/`.

| Font | Files | License | Upstream |
|---|---|---|---|
| Lilex (editor) | Regular, Bold, Italic, BoldItalic (`.ttf`), from release asset `Lilex.zip` | OFL-1.1 (`LICENSE-Lilex`) | https://github.com/mishamyrt/Lilex, tag `2.700` |
| IBM Plex Sans (UI) | Regular, Medium (`.ttf`), from release asset `ibm-plex-sans.zip` | OFL-1.1 (`LICENSE-IBM-Plex`) | https://github.com/IBM/plex, tag `@ibm/plex-sans@1.1.0` |

## Icons

`lucerna-workbench` embeds icons from Lucide (https://github.com/lucide-icons/lucide),
tag `1.70.0`, with `include_bytes!`. Each file is the upstream `icons/<name>.svg`,
unmodified, in `crates/lucerna-workbench/assets/icons/`. Lucide is licensed under
ISC; the icons it lists as derived from Feather are also under MIT (Copyright (c)
2013-present Cole Bemis). The license text is copied verbatim to
`crates/lucerna-workbench/assets/icons/LICENSE-lucide`.

Icons: `folder`, `folder-open`, `file`, `file-code`, `braces`, `book-open`,
`settings`, `lock`, `git-branch`, `scale`, `terminal`, `image`, `search`, `files`,
`x`, `chevron-right`, `chevron-down`, `chevron-up`, `blocks`, `loader-circle`.

## VS Code editing logic

Parts of the editing core in `lucerna-editor` are adapted from VS Code
(https://github.com/microsoft/vscode, MIT, Copyright (c) Microsoft Corporation).
Each file carries a comment of the form
`// Adapted from microsoft/vscode <path> (MIT)`. The files and their VS Code sources:

| Lucerna file | VS Code source |
|---|---|
| `crates/lucerna-editor/src/undo_stops.rs` | `src/vs/editor/common/cursor/cursorTypeEditOperations.ts` |
| `crates/lucerna-editor/src/words.rs` | `src/vs/editor/common/core/wordCharacterClassifier.ts`, `src/vs/editor/common/cursor/cursorWordOperations.ts` |
| `crates/lucerna-editor/src/commands.rs` | `src/vs/editor/common/cursor/cursorDeleteOperations.ts`, `src/vs/editor/common/cursor/cursorTypeEditOperations.ts`, `src/vs/editor/contrib/wordOperations/browser/wordOperations.ts`, `src/vs/editor/common/commands/shiftCommand.ts` |
| `crates/lucerna-editor/src/clipboard.rs` | `src/vs/editor/common/cursor/cursorTypeEditOperations.ts` (paste), `src/vs/editor/common/cursor/cursorDeleteOperations.ts` (cut), `src/vs/editor/common/viewModel/viewModelImpl.ts` (copy) |

```
MIT License

Copyright (c) 2015 - present Microsoft Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## VSCodeVim test cases

Some unit tests in `lucerna-vim` are adapted from the test suite of VSCodeVim
(https://github.com/VSCodeVim/Vim), which is MIT licensed. Only test cases are
adapted: no VSCodeVim source code is included.
Each adapted test carries a comment of the form
`// Adapted from VSCodeVim <file>: "<test title>"`. The cases came from these
VSCodeVim test files:

- `test/motion.test.ts`
- `test/marks.test.ts`
- `test/search/search.test.ts`
- `test/mode/modeNormal.test.ts`
- `test/mode/modeVisualLine.test.ts`
- `test/mode/normalModeTests/commands.test.ts`
- `test/mode/normalModeTests/motions.test.ts`
- `test/mode/normalModeTests/matchingBracket.test.ts`
- `test/register/repeatableMovement.test.ts`

```
MIT License

Copyright (c) 2015 VSCodeVim

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

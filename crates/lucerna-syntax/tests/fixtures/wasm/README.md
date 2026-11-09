# WASM grammar fixture

`json.wasm` is `tree-sitter-json` **0.24.8** (`src/parser.c` from the crate in
`Cargo.lock`) compiled with wasi-sdk 34.0 (macOS arm64):

```
<wasi-sdk>/bin/clang -fPIC -shared -Os -Wl,--export=tree_sitter_json -Wl,--max-memory=4294967296 -o json.wasm -I <crate>/src <crate>/src/parser.c
```

`LICENSE-tree-sitter-json` (MIT) is taken from the upstream repo at tag
v0.24.8; the published crate ships no LICENSE file.

## wasi-sdk 34.0 tarball SHA-256

| Tarball | SHA-256 |
|---|---|
| `wasi-sdk-34.0-arm64-macos.tar.gz` | `9c59398106b417f8f14913380fdf0097a8cc0ff4af9eb3ce0065a859e88d49e9` |
| `wasi-sdk-34.0-x86_64-macos.tar.gz` | `87d27fa8adc68dee59bfbf2e22a6d34ef717c34d6bf1d8af2a56fc929d9ce0eb` |

arm64 hashed locally (matches the GitHub release digest); x86_64 read from the
release asset `digest` field (`gh api repos/WebAssembly/wasi-sdk/releases/tags/wasi-sdk-34`).

## Cost of enabling tree-sitter `wasm` (wasmtime 48)

- `luc` debug binary: 70.7 MB -> 75.4 MB (+4.7 MB).
- `target/` after a full `cargo test --workspace` from a clean checkout: 3.9 GB
  (4.8 GB after also building the `luc` bin). There was no `target/` before
  this task, so an exact delta is not available.
- Building `wasmtime-c-api-impl` needs `cmake` on PATH (`brew install cmake`).

# basalt-plugin-parser-gleam

Tree-sitter Gleam parser plugin for Basalt.

## Capabilities

- `SYNTAX_PARSER`: syntax highlighting and scope extraction for `*.gleam`.
- `parse.syntax@gleam/v1`: keyword/string/number/comment/type/function spans.
- `parse.retrieval@gleam/v1`: retrieval chunks for functions, types, data
  constructors, constants, and imports.
- `parse.call-sites@gleam/v1`: bare calls, field calls (`list.map`), and
  pipeline targets (`|> string.trim`).

## Building

Parsers target `wasm32-wasip1` because they load into core's WASI-enabled
parser linker. tree-sitter 0.23 needs a WASI sysroot and the `dup()` stub:

```bash
CC_wasm32_wasip1=/path/to/wasi-sdk/bin/clang \
AR_wasm32_wasip1=/path/to/wasi-sdk/bin/llvm-ar \
CFLAGS_wasm32_wasip1="--sysroot=/path/to/wasi-sdk/share/wasi-sysroot -include $PWD/wasi_dup_stub.h" \
cargo build --release --target wasm32-wasip1
```

The artifact is `target/wasm32-wasip1/release/basalt_plugin_parser_gleam.wasm`;
publish it as `gleam.wasm`.

## Tests

```bash
cargo test --target x86_64-pc-windows-msvc   # or your host triple
cargo test --target <host> -- --ignored      # parse real add-helper sources
```

# basalt-plugin-parser-rust

Basalt syntax parser plugin for Rust (`.rs`) files.

Provides tree-sitter-based syntax highlighting, semantic retrieval chunks, and call-site extraction for Rust source files via the Basalt parser plugin ABI (`basalt_lang`, `basalt_parse`, `basalt_retrieval_chunks`, `basalt_call_sites`).

## Installation

Install via the Basalt plugin registry, or manually:

```bash
curl -L https://github.com/adevcorn/basalt-plugin-parser-rust/releases/latest/download/parser.wasm \
  -o ~/.config/basalt/parsers/rust.wasm
```

## Building from source

```bash
rustup target add wasm32-wasip1
export CC_wasm32_wasip1=/path/to/clang
export AR_wasm32_wasip1=/path/to/llvm-ar
export CFLAGS_wasm32_wasip1="--sysroot=/path/to/wasi-sysroot -include wasi_dup_stub.h"
cargo build --release --target wasm32-wasip1
cp target/wasm32-wasip1/release/basalt_plugin_parser_rust.wasm ~/.config/basalt/parsers/rust.wasm
```

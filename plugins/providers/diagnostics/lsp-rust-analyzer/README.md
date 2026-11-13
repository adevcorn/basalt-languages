# basalt-plugin-lsp-rust-analyzer

Basalt plugin: Rust Analyzer LSP bridge — diagnostics, hover, and completions for Rust projects.

## Installation

Download the latest `.wasm` from [Releases](https://github.com/adevcorn/basalt-plugin-lsp-rust-analyzer/releases) and place it in `~/.config/basalt/plugins/`.

Or install via the Basalt plugin registry.

## Building from source

```bash
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/lsp_rust_analyzer.wasm ~/.config/basalt/plugins/
```

# basalt-plugin-rust-review-actions

Basalt plugin: Rust review-time actions.

This plugin exposes the `review-action:test:rust` capability so Basalt can run `cargo test` against speculative Rust workspaces before approval.

## Installation

Download the latest `.wasm` from [Releases](https://github.com/adevcorn/basalt-plugin-rust-review-actions/releases) and place it in `~/.config/basalt/plugins/`.

Or install via the Basalt plugin registry.

## Building from source

```bash
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/rust_review_actions.wasm ~/.config/basalt/plugins/
```

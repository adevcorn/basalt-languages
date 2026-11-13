# basalt-plugin-semantic-retrieval

Basalt plugin: opt-in semantic retrieval capability.

Installing this plugin enables host-side retrieval features in Basalt, including:

- workspace embedding index builds
- launch grounding
- semantic command palette ranking
- related-file suggestions in review

## Installation

Download the latest `.wasm` from [Releases](https://github.com/adevcorn/basalt-plugin-semantic-retrieval/releases) and place it in `~/.config/basalt/plugins/`.

Or install via the Basalt plugin registry.

## Building from source

```bash
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/semantic_retrieval.wasm ~/.config/basalt/plugins/semantic-retrieval.wasm
```

# basalt-languages

Language packs, providers, and presets for Basalt.

This is the Phase 1 consolidation monorepo. It contains every language-related
plugin artifact and the manifests that group them:

- **Language packs** (`packs/`) own the parser and parser-derived semantic facts:
  `language-csharp`, `language-kotlin`, `language-gleam`, `language-typescript`,
  `language-yaml`, `language-rust`, `language-swift`, `language-c`,
  `language-markdown`, `language-toml`.
- **Providers** (`plugins/providers/`) own toolchain/workflow concerns: project
  models, API indexes, review actions, LSP bridges, and framework add-ons.
- **Presets** (`presets/`) are curated install groups that pin packs and providers,
  e.g. `dotnet`, `kotlin-android`, `gleam`, `web`, `rust`, `swift`.

The ownership contract and migration plan live in
[`../LANGUAGE_PLUGIN_CONSOLIDATION_PLAN.md`](../LANGUAGE_PLUGIN_CONSOLIDATION_PLAN.md).

## Layout

```
crates/language-core/      shared ABI constants and helpers
crates/testkit/            golden-corpus helpers
plugins/parser/<lang>/     parser artifacts (parser ABI, WASI-only)
plugins/facts/<lang>/      parser-derived semantic-facts artifacts
plugins/providers/         project models, API index, review actions, LSP, add-ons
packs/                     language pack manifests
presets/                   curated install groups (+ auto-install markers)
release/artifacts.toml     the complete build list used by CI
scripts/                   build and collection helpers
```

## Building

Local (macOS Homebrew LLVM + wasi-libc):

```bash
./scripts/build-wasm.sh                 # all artifacts -> dist/
./scripts/build-wasm.sh -p basalt-plugin-parser-kotlin
```

With a wasi-sdk checkout:

```bash
WASI_SDK=/path/to/wasi-sdk ./scripts/build-wasm.sh
```

`WASI_SDK` must contain `bin/clang`, `bin/llvm-ar`, and
`share/wasi-sysroot`. Without it the script auto-detects Homebrew's
`llvm` + `wasi-libc`, or uses `WASI_SYSROOT` and `CC`/`AR` from the environment.

Validate the pack/preset/artifact manifests:

```bash
python3 scripts/validate_manifests.py
```

Golden corpus (requires built artifacts):

```bash
./scripts/build-wasm.sh
GOLDEN_DIST="$PWD/dist" cargo test -p golden --test corpus
```

Regenerate snapshots after an intentional change:

```bash
GOLDEN_UPDATE=1 GOLDEN_DIST="$PWD/dist" cargo test -p golden --test corpus
```

`crates/golden` loads the real `dist/*.wasm` artifacts in wasmtime and compares
scope spans, retrieval chunks (including v2 flags), and call sites for all ten
languages. CI runs it after the wasm build matrix.

Host tests:

```bash
cargo test --workspace --exclude golden
```

## CI

- `ci.yml` runs host tests and builds every artifact in a matrix generated from
  `release/artifacts.toml`.
- `release.yml` builds the same matrix and attaches all `.wasm` files to a
  `v*` GitHub release.

## Registry

`scripts/generate_registry.py` produces registry entries for the language packs,
presets, and providers. Packs and presets are emitted into separate top-level
`packs`/`presets` arrays so older clients ignore them. It points artifact URLs at
the monorepo release `--tag` and includes SHA-256 checksums from `--dist`
(download the actual release assets, not the local build, because CI builds are
not byte-identical to local builds):

```bash
gh release download v0.1.1 --repo adevcorn/basalt-languages -D /tmp/relassets
python3 scripts/generate_registry.py \
  --merge ../basalt-registry/index.json \
  --dist /tmp/relassets \
  --out /tmp/index.next.json
```

The live registry is updated separately once a core release can consume
`kind: "preset"` entries.

## Provenance

`sources.json` records the original repository and commit for every imported
crate. Git history grafting is a follow-up; Phase 1 preserves source contents and
artifact names. Each imported crate keeps its original license (see its
`Cargo.toml`/README). The standalone `basalt-plugin-semantic-facts-rust` was
imported instead of the stale in-tree copy under `basalt/`.

## Notes

- The SDK is pinned to `adevcorn/basalt-plugin-sdk` tag `v0.1.6` as a workspace
  dependency. Crates that previously used older tags/revs now share it.
- Per-crate release profiles were replaced by one workspace profile. Parsers keep
  `opt-level = "z"`; providers may grow slightly compared with their old `"s"`.
- Parsers keep their original filenames (`cs.wasm`, `kt.wasm`, `gleam.wasm`,
  `ts.wasm`, `yaml.wasm`, `rs.wasm`, `swift.wasm`, `c.wasm`, `md.wasm`,
  `toml.wasm`) so existing `ParserRegistry` discovery keeps working.
- `semantic-facts-typescript` gained a host-only `invoke_capability` stub so
  `cargo check`/`cargo test` works off-wasm; the wasm build uses the SDK's real
  host-capability import.
- `dotnet-api-index` tests that pointed at the author's absolute checkout now
  skip when those paths are absent, so the suite is hermetic in CI.
- Shared-code extraction into `language-core` and the golden corpus are still
  Phase 1 follow-ups; the crates currently keep their original implementations.

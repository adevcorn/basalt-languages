# basalt-plugin-parser-typescript

Tree-sitter TypeScript/TSX parser WASM plugin for Basalt.

## File extensions
- `.ts`, `.mts`, `.cts` — TypeScript grammar
- `.tsx`, `.js`, `.jsx`, `.mjs`, `.cjs` — TSX grammar (JS + JSX)

## Exports (WASM ABI)
- `basalt_lang` — returns pointer to `"ts\0"`
- `basalt_lang_tsx` — returns pointer to `"tsx\0"`
- `basalt_ext_aliases` — `alias:canonical` map for the whole ECMAScript family
- `basalt_ext_dialects` — `extension:dialect` map (`tsx` → `tsx`, `js` → `tsx`)
- `basalt_src_ptr` / `basalt_out_ptr` — shared memory buffer pointers
- `basalt_parse(src_ptr, src_len, out_ptr, max_spans) -> i32` — syntax highlight spans
- `basalt_parse_tsx(...)` — TSX variant
- `basalt_retrieval_chunks(...)` / `basalt_retrieval_chunks_tsx(...)`
- `basalt_call_sites(...)` / `basalt_call_sites_tsx(...)`

Hosts that support dialects call `basalt_<abi>_<dialect>` per file extension and
fall back to the unsuffixed export when it is absent.
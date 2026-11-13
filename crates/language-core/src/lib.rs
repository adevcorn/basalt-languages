//! Shared runtime surface for Basalt language packs.
//!
//! Phase 1 keeps this intentionally small: the per-language crates still own
//! their ABI boilerplate. Subsequent phases move the wire-format helpers,
//! retrieval-chunk v2 encoding, lexical-profile plumbing, and facts scaffolding
//! here so parsers and facts modules share one implementation.

/// Version of the plugin ABI these artifacts are built against.
pub const ABI_VERSION: u32 = 1;

/// Wire sizes shared by the parser ABI and the host.
pub mod wire {
    /// `offset u32 + length u32 + scope u8 + 3 reserved`
    pub const SCOPE_SPAN_BYTES: usize = 12;
    /// `offset u32 + length u32 + label[95] + kind u8`
    pub const RETRIEVAL_CHUNK_BYTES_V1: usize = 104;
    /// v1 plus `flags u8 + reserved u8`
    pub const RETRIEVAL_CHUNK_BYTES_V2: usize = 106;
    /// `offset u32 + callee[64]`
    pub const CALL_SITE_BYTES: usize = 68;
}

/// Canonical extension for an artifact name (e.g. `cs.wasm` -> `cs`).
pub fn extension_from_artifact(name: &str) -> &str {
    name.strip_suffix(".wasm").unwrap_or(name)
}

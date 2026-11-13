//! Golden-corpus helpers for language-pack regression tests.
//!
//! The Phase 0 corpus captures normalized parser and facts output per language.
//! These helpers keep the comparison rules in one place so every crate's tests
//! fail the same way when output drifts.

pub use language_core::extension_from_artifact;

/// Normalize a retrieval label for stable golden comparisons.
///
/// Collapses runs of whitespace and trims, but preserves the label prefix
/// (`function`, `type`, ...) and any symbol name.
pub fn normalize_label(label: &str) -> String {
    label.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Normalize a sorted list of `(kind, label)` chunks.
pub fn normalize_chunks(chunks: &[(u8, String)]) -> Vec<(u8, String)> {
    let mut out: Vec<(u8, String)> = chunks
        .iter()
        .map(|(kind, label)| (*kind, normalize_label(label)))
        .collect();
    out.sort();
    out.dedup();
    out
}

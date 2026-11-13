//! Shared runtime surface for Basalt language packs.
//!
//! The parser ABI is intentionally tiny: fixed-size records written into the
//! guest's output buffer. These helpers keep the record layout and scope/kind
//! mappings in one place so parser crates do not copy the same byte-packing
//! code.

#![no_std]

extern crate alloc;

/// Version of the plugin ABI these artifacts are built against.
pub const ABI_VERSION: u32 = 1;

pub mod wire {
    /// `offset u32 + length u32 + scope u8 + 3 reserved`
    pub const SCOPE_SPAN_BYTES: usize = 12;
    /// `offset u32 + length u32 + label[95] + kind u8`
    pub const RETRIEVAL_CHUNK_BYTES_V1: usize = 104;
    /// v1 plus `flags u8 + reserved u8`
    pub const RETRIEVAL_CHUNK_BYTES_V2: usize = 106;
    /// `offset u32 + callee[64]`
    pub const CALL_SITE_BYTES: usize = 68;

    pub const SCOPE_KEYWORD: u8 = 1;
    pub const SCOPE_STRING: u8 = 2;
    pub const SCOPE_NUMBER: u8 = 3;
    pub const SCOPE_COMMENT: u8 = 4;
    pub const SCOPE_TYPE: u8 = 5;
    pub const SCOPE_FUNCTION: u8 = 6;
    pub const SCOPE_OPERATOR: u8 = 7;
    pub const SCOPE_MACRO: u8 = 8;
    pub const SCOPE_VARIABLE: u8 = 10;
    pub const SCOPE_NAMESPACE: u8 = 11;

    /// Map a tree-sitter capture suffix to a scope id.
    pub fn scope_id_for(name: &str) -> u8 {
        match name {
            "keyword" => SCOPE_KEYWORD,
            "string" => SCOPE_STRING,
            "number" => SCOPE_NUMBER,
            "comment" => SCOPE_COMMENT,
            "type" => SCOPE_TYPE,
            "function" => SCOPE_FUNCTION,
            "operator" => SCOPE_OPERATOR,
            "macro" => SCOPE_MACRO,
            "variable" => SCOPE_VARIABLE,
            "module" | "namespace" => SCOPE_NAMESPACE,
            _ => 0,
        }
    }

    /// Map a retrieval chunk kind to the semantic-node byte.
    pub fn kind_byte(kind: &str) -> u8 {
        match kind {
            "module" => 1,
            "type" => 2,
            "function" => 3,
            "extension" => 4,
            "property" => 5,
            "enum" | "enum_case" => 6,
            "interface" => 7,
            _ => 0,
        }
    }

    pub fn write_span(out: &mut [u8], index: usize, offset: u32, length: u32, scope: u8) {
        let base = index * SCOPE_SPAN_BYTES;
        out[base..base + 4].copy_from_slice(&offset.to_le_bytes());
        out[base + 4..base + 8].copy_from_slice(&length.to_le_bytes());
        out[base + 8] = scope;
        out[base + 9] = 0;
        out[base + 10] = 0;
        out[base + 11] = 0;
    }

    /// Write a v1 retrieval chunk (`kind` at byte 103).
    pub fn write_chunk(
        out: &mut [u8],
        index: usize,
        offset: u32,
        length: u32,
        label: &str,
        kind: u8,
    ) {
        write_chunk_with_flags(out, index, offset, length, label, kind, 0, false);
    }

    /// Write a v2 retrieval chunk (`flags` at byte 104).
    pub fn write_chunk_v2(
        out: &mut [u8],
        index: usize,
        offset: u32,
        length: u32,
        label: &str,
        kind: u8,
        flags: u8,
    ) {
        write_chunk_with_flags(out, index, offset, length, label, kind, flags, true);
    }

    fn write_chunk_with_flags(
        out: &mut [u8],
        index: usize,
        offset: u32,
        length: u32,
        label: &str,
        kind: u8,
        flags: u8,
        v2: bool,
    ) {
        let stride = if v2 {
            RETRIEVAL_CHUNK_BYTES_V2
        } else {
            RETRIEVAL_CHUNK_BYTES_V1
        };
        let base = index * stride;
        out[base..base + 4].copy_from_slice(&offset.to_le_bytes());
        out[base + 4..base + 8].copy_from_slice(&length.to_le_bytes());
        let label_bytes = label.as_bytes();
        let label_len = label_bytes.len().min(95);
        out[base + 8..base + 8 + label_len].copy_from_slice(&label_bytes[..label_len]);
        out[base + 8 + label_len] = 0;
        out[base + 103] = kind;
        if v2 {
            out[base + 104] = flags;
            out[base + 105] = 0;
        }
    }

    pub fn write_call_site(out: &mut [u8], index: usize, offset: u32, name: &str) {
        let base = index * CALL_SITE_BYTES;
        out[base..base + 4].copy_from_slice(&offset.to_le_bytes());
        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len().min(63);
        out[base + 4..base + 4 + name_len].copy_from_slice(&name_bytes[..name_len]);
        out[base + 4 + name_len] = 0;
    }

    /// Canonical extension for an artifact name (e.g. `cs.wasm` -> `cs`).
    pub fn extension_from_artifact(name: &str) -> &str {
        name.strip_suffix(".wasm").unwrap_or(name)
    }
}

pub mod facts_wire {
    //! Decoders for the parser ABI records consumed by semantic-facts plugins.

    use alloc::vec::Vec;

    /// Decode `basalt_call_sites` output (`offset u32 + callee[64]`).
    pub fn decode_call_sites(data: &[u8]) -> Vec<(u32, &str)> {
        if data.len() < 4 {
            return Vec::new();
        }
        let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let mut out = Vec::with_capacity(count);
        let mut pos = 4;
        for _ in 0..count {
            if pos + 68 > data.len() {
                break;
            }
            let offset = u32::from_le_bytes([
                data[pos],
                data[pos + 1],
                data[pos + 2],
                data[pos + 3],
            ]);
            let name = read_cstr(&data[pos + 4..pos + 68]);
            if !name.is_empty() {
                out.push((offset, name));
            }
            pos += 68;
        }
        out
    }

    /// Decode retrieval chunks (`offset u32 + length u32 + label[95] + kind u8`).
    pub fn decode_retrieval(data: &[u8]) -> Vec<(u32, u32, &str, u8)> {
        if data.len() < 4 {
            return Vec::new();
        }
        let count = u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as usize;
        let mut out = Vec::with_capacity(count);
        let mut pos = 4;
        for _ in 0..count {
            if pos + 104 > data.len() {
                break;
            }
            let offset = u32::from_le_bytes([
                data[pos],
                data[pos + 1],
                data[pos + 2],
                data[pos + 3],
            ]);
            let length = u32::from_le_bytes([
                data[pos + 4],
                data[pos + 5],
                data[pos + 6],
                data[pos + 7],
            ]);
            let label = read_cstr(&data[pos + 8..pos + 103]);
            let kind = data[pos + 103];
            if !label.is_empty() && length > 0 {
                out.push((offset, length, label, kind));
            }
            pos += 104;
        }
        out
    }

    /// Strip the `"<kind> "` prefix from a retrieval label.
    pub fn label_name<'a>(label: &'a str, kind: &str) -> &'a str {
        label
            .strip_prefix(kind)
            .map(str::trim)
            .unwrap_or(label)
            .trim()
    }

    fn read_cstr(bytes: &[u8]) -> &str {
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        core::str::from_utf8(&bytes[..end]).unwrap_or("").trim()
    }
}

pub use wire::extension_from_artifact;

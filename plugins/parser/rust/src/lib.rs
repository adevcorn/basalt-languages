// rust-parser/src/lib.rs — tree-sitter Rust parser WASM plugin for Basalt
//
// Memory layout (within the plugin's own MEMORY buffer):
//   MEMORY[SRC_OFFSET..] — host writes source bytes here
//   MEMORY[OUT_OFFSET..] — plugin writes output records here
//
// The host must call basalt_src_ptr() / basalt_out_ptr() after instantiation
// to obtain the linear-memory addresses to write/read.  Do NOT use hardcoded
// offsets into linear memory — the WASM data segment may overlap them.
//
// Exports:
//   basalt_lang()                     → i32 ptr to "rs\0"
//   basalt_src_ptr()                  → i32 linear-memory address for source input
//   basalt_out_ptr()                  → i32 linear-memory address for output
//   basalt_parse(src,len,out,max)     → i32 span count   (12-byte ScopeSpan)
//   basalt_retrieval_chunks(...)      → i32 chunk count  (104-byte RetrievalChunk)
//   basalt_call_sites(...)            → i32 site count   (68-byte CallSite)

use tree_sitter::{Language, Parser, Query, QueryCursor};
use language_core::wire;

const SRC_OFFSET: usize = 0;
const OUT_OFFSET: usize = 6 * 1024 * 1024;
const MEMORY_BYTES: usize = 12 * 1024 * 1024;

const SCOPE_KEYWORD:   u8 = 1;
const SCOPE_STRING:    u8 = 2;
const SCOPE_NUMBER:    u8 = 3;
const SCOPE_COMMENT:   u8 = 4;
const SCOPE_TYPE:      u8 = 5;
const SCOPE_FUNCTION:  u8 = 6;
const SCOPE_OPERATOR:  u8 = 7;
const SCOPE_MACRO:     u8 = 8;
const SCOPE_VARIABLE:  u8 = 10;
const SCOPE_NAMESPACE: u8 = 11;

static mut MEMORY: [u8; MEMORY_BYTES] = [0u8; MEMORY_BYTES];
static LANG_EXT: &[u8] = b"rs\0";

extern "C" { fn tree_sitter_rust() -> Language; }

// ---------------------------------------------------------------------------
// Static parser + query cache (WASM is single-threaded; safe to use static mut)
// ---------------------------------------------------------------------------

struct ParserState {
    parser: Parser,
    parse_query: Query,
    retrieval_query: Query,
    call_sites_query: Query,
    parse_cap_names: Vec<String>,
    retrieval_cap_names: Vec<String>,
}

static mut STATE: Option<ParserState> = None;

unsafe fn get_state() -> Option<&'static mut ParserState> {
    if STATE.is_none() {
        let lang = tree_sitter_rust();
        let mut parser = Parser::new();
        parser.set_language(lang).ok()?;

        let parse_query_src = r#"
            "fn" @keyword "let" @keyword "pub" @keyword "struct" @keyword
            "enum" @keyword "impl" @keyword "trait" @keyword "use" @keyword
            "mod" @keyword "if" @keyword "else" @keyword "for" @keyword
            "while" @keyword "loop" @keyword "return" @keyword "match" @keyword
            "type" @keyword "where" @keyword "async" @keyword "await" @keyword
            "unsafe" @keyword "extern" @keyword "dyn" @keyword "move" @keyword
            "ref" @keyword "static" @keyword "const" @keyword "default" @keyword
            "in" @keyword "as" @keyword "break" @keyword "continue" @keyword
            (mutable_specifier) @keyword
            (string_literal) @string (raw_string_literal) @string
            (char_literal) @string
            (boolean_literal) @keyword
            (integer_literal) @number (float_literal) @number
            (line_comment) @comment (block_comment) @comment
            (type_identifier) @type (primitive_type) @type
            (function_item name: (identifier) @function)
            (function_signature_item name: (identifier) @function)
            (call_expression
              function: (identifier) @function)
            (call_expression
              function: (scoped_identifier name: (identifier) @function))
            (call_expression
              function: (field_expression field: (field_identifier) @function))
            (generic_function
              function: (identifier) @function)
            (generic_function
              function: (scoped_identifier name: (identifier) @function))
            (macro_invocation macro: (identifier) @macro "!" @macro)
            (macro_invocation macro: (scoped_identifier name: (identifier) @macro) "!" @macro)
            (macro_definition name: (identifier) @macro)
            (attribute_item) @macro
            (inner_attribute_item) @macro
            "->" @operator "=>" @operator "==" @operator "!=" @operator
            "<=" @operator ">=" @operator "&&" @operator "||" @operator
            "+=" @operator "-=" @operator "*=" @operator "/=" @operator
            "&=" @operator "|=" @operator "^=" @operator ">>=" @operator "<<=" @operator
            "%" @operator "&" @operator "|" @operator "^" @operator
            "<<" @operator ">>" @operator ".." @operator "..=" @operator "@" @operator
            (let_declaration pattern: (identifier) @variable)
            (let_declaration pattern: (tuple_pattern (identifier) @variable))
            (parameter pattern: (identifier) @variable)
            (closure_parameters (identifier) @variable)
            (scoped_identifier path: (identifier) @namespace)
            (scoped_type_identifier path: (identifier) @namespace)
            (use_declaration argument: (identifier) @namespace)
            (mod_item name: (identifier) @namespace)
            (extern_crate_declaration name: (identifier) @namespace)
        "#;
        let parse_query = Query::new(lang, parse_query_src).ok()?;
        let parse_cap_names: Vec<String> = parse_query.capture_names().iter().map(|s| s.to_string()).collect();

        let retrieval_query_src = r#"
            (function_item name: (identifier) @name.function) @chunk.function
            (struct_item name: (type_identifier) @name.type) @chunk.type
            (enum_item name: (type_identifier) @name.type) @chunk.type
            (trait_item name: (type_identifier) @name.type) @chunk.type
            (impl_item type: (_) @name.extension) @chunk.extension
            (mod_item name: (identifier) @name.module) @chunk.module
        "#;
        let retrieval_query = Query::new(lang, retrieval_query_src).ok()?;
        let retrieval_cap_names: Vec<String> = retrieval_query.capture_names().iter().map(|s| s.to_string()).collect();

        let call_sites_query_src = r#"
            (call_expression function: (identifier) @callee)
            (call_expression function: (field_expression field: (field_identifier) @callee))
            (call_expression function: (scoped_identifier name: (identifier) @callee))
        "#;
        let call_sites_query = Query::new(lang, call_sites_query_src).ok()?;

        STATE = Some(ParserState {
            parser,
            parse_query,
            retrieval_query,
            call_sites_query,
            parse_cap_names,
            retrieval_cap_names,
        });
    }
    STATE.as_mut()
}

// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn basalt_lang() -> i32 {
    LANG_EXT.as_ptr() as i32
}

/// Returns the linear-memory address the host should write source bytes into.
#[no_mangle]
pub unsafe extern "C" fn basalt_src_ptr() -> i32 {
    MEMORY[SRC_OFFSET..].as_ptr() as i32
}

/// Returns the linear-memory address the host should read output records from.
#[no_mangle]
pub unsafe extern "C" fn basalt_out_ptr() -> i32 {
    MEMORY[OUT_OFFSET..].as_ptr() as i32
}

#[no_mangle]
pub unsafe extern "C" fn basalt_parse(
    src_ptr: i32, src_len: i32, out_ptr: i32, max_spans: i32,
) -> i32 {
    let src = std::slice::from_raw_parts(
        src_ptr as usize as *const u8,
        src_len as usize,
    );
    let out = std::slice::from_raw_parts_mut(
        out_ptr as usize as *mut u8,
        (max_spans as usize) * 12,
    );

    let state = match get_state() { Some(s) => s, None => return 0 };
    state.parser.reset();
    let Some(tree) = state.parser.parse(src, None) else { return 0 };

    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&state.parse_query, tree.root_node(), src);

    let mut count = 0usize;
    for m in matches {
        for cap in m.captures {
            if count >= max_spans as usize { break; }
            let scope_id = wire::scope_id_for(&state.parse_cap_names[cap.index as usize]);
            let offset = cap.node.start_byte() as u32;
            let length = (cap.node.end_byte() - cap.node.start_byte()) as u32;
            wire::write_span(out, count, offset, length, scope_id);
            count += 1;
        }
    }
    count as i32
}

/// Map the tree-sitter capture kind string to the Basalt semantic node kind byte.
/// These values must match the `SEMANTIC_NODE_*` constants in `core/src/semantic_layer.rs`.
fn kind_byte(k: &str) -> u8 { wire::kind_byte(k) }

#[no_mangle]
pub unsafe extern "C" fn basalt_retrieval_chunks(
    src_ptr: i32, src_len: i32, out_ptr: i32, max_chunks: i32,
) -> i32 {
    let src = std::slice::from_raw_parts(
        src_ptr as usize as *const u8,
        src_len as usize,
    );
    let out = std::slice::from_raw_parts_mut(
        out_ptr as usize as *mut u8,
        (max_chunks as usize) * 104,
    );

    let state = match get_state() { Some(s) => s, None => return 0 };
    state.parser.reset();
    let Some(tree) = state.parser.parse(src, None) else { return 0 };

    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&state.retrieval_query, tree.root_node(), src);

    let mut count = 0usize;
    for m in matches {
        if count >= max_chunks as usize { break; }
        let mut offset = None::<u32>;
        let mut length = None::<u32>;
        let mut kind = None::<&str>;
        let mut name = None::<&str>;
        for cap in m.captures {
            let cn = &state.retrieval_cap_names[cap.index as usize];
            if let Some(k) = cn.strip_prefix("chunk.") {
                offset = Some(cap.node.start_byte() as u32);
                length = Some((cap.node.end_byte() - cap.node.start_byte()) as u32);
                kind = Some(k);
            } else if cn.starts_with("name.") {
                if let Ok(t) = cap.node.utf8_text(src) { name = Some(t.trim()); }
            }
        }
        let (Some(off), Some(len), Some(k)) = (offset, length, kind) else { continue };
        let label = if let Some(n) = name {
            let mut s = k.to_string(); s.push(' '); s.push_str(n); s
        } else { k.to_string() };
        wire::write_chunk(out, count, off, len, &label, kind_byte(k));
        count += 1;
    }
    count as i32
}

#[no_mangle]
pub unsafe extern "C" fn basalt_call_sites(
    src_ptr: i32, src_len: i32, out_ptr: i32, max_sites: i32,
) -> i32 {
    let src = std::slice::from_raw_parts(
        src_ptr as usize as *const u8,
        src_len as usize,
    );
    let out = std::slice::from_raw_parts_mut(
        out_ptr as usize as *mut u8,
        (max_sites as usize) * 68,
    );

    let state = match get_state() { Some(s) => s, None => return 0 };
    state.parser.reset();
    let Some(tree) = state.parser.parse(src, None) else { return 0 };

    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&state.call_sites_query, tree.root_node(), src);

    let mut count = 0usize;
    for m in matches {
        if count >= max_sites as usize { break; }
        for cap in m.captures {
            let Ok(name) = cap.node.utf8_text(src) else { continue };
            let name = name.trim();
            if name.is_empty() { continue; }
            let offset = cap.node.start_byte() as u32;
            wire::write_call_site(out, count, offset, name);
            count += 1;
        }
    }
    count as i32
}

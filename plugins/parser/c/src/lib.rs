// c-parser/src/lib.rs — tree-sitter C parser WASM plugin for Basalt
//
// The host must call basalt_src_ptr() / basalt_out_ptr() after instantiation
// to obtain the linear-memory addresses to write/read.

use tree_sitter::{Language, Parser, Query, QueryCursor};

const SRC_OFFSET: usize = 0;
const OUT_OFFSET: usize = 6 * 1024 * 1024;
const MEMORY_BYTES: usize = 12 * 1024 * 1024;

const SCOPE_KEYWORD: u8 = 1;
const SCOPE_STRING: u8 = 2;
const SCOPE_NUMBER: u8 = 3;
const SCOPE_COMMENT: u8 = 4;
const SCOPE_TYPE: u8 = 5;
const SCOPE_FUNCTION: u8 = 6;
const SCOPE_OPERATOR: u8 = 7;
const SCOPE_MACRO: u8 = 8;

static mut MEMORY: [u8; MEMORY_BYTES] = [0u8; MEMORY_BYTES];
static LANG_EXT: &[u8] = b"c\0";

extern "C" { fn tree_sitter_c() -> Language; }

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
        let lang = tree_sitter_c();
        let mut parser = Parser::new();
        parser.set_language(lang).ok()?;

        let parse_query_src = r#"
            "if" @keyword "else" @keyword "for" @keyword "while" @keyword "do" @keyword
            "return" @keyword "struct" @keyword "enum" @keyword "typedef" @keyword
            "static" @keyword "const" @keyword "extern" @keyword "sizeof" @keyword
            (primitive_type) @type
            (string_literal) @string (char_literal) @string
            (number_literal) @number (comment) @comment
            (type_identifier) @type
            (function_declarator declarator: (identifier) @function)
            (call_expression function: (identifier) @function)
            (preproc_directive) @macro
        "#;
        let parse_query = Query::new(lang, parse_query_src).ok()?;
        let parse_cap_names: Vec<String> = parse_query.capture_names().iter().map(|s| s.to_string()).collect();

        let retrieval_query_src = r#"
            (function_definition declarator: (function_declarator declarator: (identifier) @name.function)) @chunk.function
            (struct_specifier name: (type_identifier) @name.type) @chunk.type
            (enum_specifier name: (type_identifier) @name.type) @chunk.type
            (union_specifier name: (type_identifier) @name.type) @chunk.type
        "#;
        let retrieval_query = Query::new(lang, retrieval_query_src).ok()?;
        let retrieval_cap_names: Vec<String> = retrieval_query.capture_names().iter().map(|s| s.to_string()).collect();

        let call_sites_query_src = r#"(call_expression function: (identifier) @callee)"#;
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

#[no_mangle]
pub unsafe extern "C" fn basalt_src_ptr() -> i32 {
    MEMORY[SRC_OFFSET..].as_ptr() as i32
}

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
            let scope_id = match state.parse_cap_names[cap.index as usize].as_str() {
                "keyword"  => SCOPE_KEYWORD,
                "string"   => SCOPE_STRING,
                "number"   => SCOPE_NUMBER,
                "comment"  => SCOPE_COMMENT,
                "type"     => SCOPE_TYPE,
                "function" => SCOPE_FUNCTION,
                "operator" => SCOPE_OPERATOR,
                "macro"    => SCOPE_MACRO,
                _ => 0,
            };
            let offset = cap.node.start_byte() as u32;
            let length = (cap.node.end_byte() - cap.node.start_byte()) as u32;
            let base = count * 12;
            out[base..base+4].copy_from_slice(&offset.to_le_bytes());
            out[base+4..base+8].copy_from_slice(&length.to_le_bytes());
            out[base+8] = scope_id;
            out[base+9] = 0; out[base+10] = 0; out[base+11] = 0;
            count += 1;
        }
    }
    count as i32
}

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
        let base = count * 104;
        out[base..base+4].copy_from_slice(&off.to_le_bytes());
        out[base+4..base+8].copy_from_slice(&len.to_le_bytes());
        let lbytes = label.as_bytes();
        let llen = lbytes.len().min(95);
        out[base+8..base+8+llen].copy_from_slice(&lbytes[..llen]);
        out[base+8+llen] = 0;
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
            let base = count * 68;
            out[base..base+4].copy_from_slice(&offset.to_le_bytes());
            let nb = name.as_bytes();
            let nlen = nb.len().min(63);
            out[base+4..base+4+nlen].copy_from_slice(&nb[..nlen]);
            out[base+4+nlen] = 0;
            count += 1;
        }
    }
    count as i32
}

static EXT_ALIASES: &[u8] = b"h:c\nhpp:c\nhh:c\0";

#[no_mangle]
pub extern "C" fn basalt_ext_aliases() -> i32 {
    EXT_ALIASES.as_ptr() as i32
}

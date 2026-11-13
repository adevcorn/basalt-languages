// basalt-plugin-parser-gleam/src/lib.rs — tree-sitter Gleam parser WASM plugin for Basalt
//
// Implements the parser ABI consumed by `basalt-core/src/parser.rs`:
//   basalt_lang, basalt_parse, basalt_retrieval_chunks, basalt_call_sites
//
// Retrieval chunks use the 104-byte v1 record layout:
//   [offset u32 LE][length u32 LE][label 95 bytes + NUL][kind u8]

#![allow(static_mut_refs)]

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
const SCOPE_VARIABLE: u8 = 10;

static mut MEMORY: [u8; MEMORY_BYTES] = [0u8; MEMORY_BYTES];
static LANG_EXT: &[u8] = b"gleam\0";

struct ParserState {
    parser: Parser,
    parse_query: Query,
    retrieval_query: Query,
    call_sites_query: Query,
    parse_cap_names: Vec<String>,
    retrieval_cap_names: Vec<String>,
}

static mut STATE: Option<ParserState> = None;

fn make_language() -> Language {
    tree_sitter_gleam::LANGUAGE.into()
}

fn make_state() -> Option<ParserState> {
    let lang = make_language();
    let mut parser = Parser::new();
    parser.set_language(&lang).ok()?;

    let parse_query_src = r#"
        [
          (visibility_modifier)
          (opacity_modifier)
          "as" "assert" "case" "const" "echo" "external"
          "fn" "if" "import" "let" "panic" "todo" "type" "use"
        ] @keyword

        (comment) @comment
        (module_comment) @comment
        (statement_comment) @comment

        (string) @string
        (integer) @number
        (float) @number

        (type_identifier) @type
        (constructor_name) @type

        (function name: (identifier) @function)
        (external_function name: (identifier) @function)
        (function_call function: (identifier) @function)
        (field_access field: (label) @function)

        (function_parameter name: (identifier) @variable)
        (constant name: (identifier) @variable)

        (binary_expression operator: _ @operator)
        (boolean_negation "!" @operator)
        (integer_negation "-" @operator)
    "#;
    let parse_query = Query::new(&lang, parse_query_src).ok()?;
    let parse_cap_names: Vec<String> = parse_query
        .capture_names()
        .iter()
        .map(|s| s.to_string())
        .collect();

    let retrieval_query_src = r#"
        (function name: (identifier) @name.function) @chunk.function
        (external_function name: (identifier) @name.function) @chunk.function
        (type_definition
          (type_name name: (type_identifier) @name.type)) @chunk.type
        (external_type
          (type_name name: (type_identifier) @name.type)) @chunk.type
        (data_constructor
          name: (constructor_name) @name.enum_case) @chunk.enum_case
        (constant name: (identifier) @name.property) @chunk.property
        (import module: (module) @name.module) @chunk.module
    "#;
    let retrieval_query = Query::new(&lang, retrieval_query_src).ok()?;
    let retrieval_cap_names: Vec<String> = retrieval_query
        .capture_names()
        .iter()
        .map(|s| s.to_string())
        .collect();

    let call_sites_query_src = r#"
        (function_call function: (identifier) @callee)
        (function_call function: (field_access field: (label) @callee))
        (binary_expression operator: "|>" right: (identifier) @callee)
        (binary_expression operator: "|>" right: (field_access field: (label) @callee))
    "#;
    let call_sites_query = Query::new(&lang, call_sites_query_src).ok()?;

    Some(ParserState {
        parser,
        parse_query,
        retrieval_query,
        call_sites_query,
        parse_cap_names,
        retrieval_cap_names,
    })
}

fn get_state() -> Option<&'static mut ParserState> {
    // SAFETY: the plugin is single-threaded (one WASM instance per call) and
    // STATE is only mutated on first use.
    unsafe {
        if STATE.is_none() {
            STATE = make_state();
        }
        STATE.as_mut()
    }
}

fn scope_id_for(name: &str) -> u8 {
    match name {
        "keyword" => SCOPE_KEYWORD,
        "string" => SCOPE_STRING,
        "number" => SCOPE_NUMBER,
        "comment" => SCOPE_COMMENT,
        "type" => SCOPE_TYPE,
        "function" => SCOPE_FUNCTION,
        "operator" => SCOPE_OPERATOR,
        "variable" => SCOPE_VARIABLE,
        _ => 0,
    }
}

fn kind_byte(kind: &str) -> u8 {
    match kind {
        "module" => 1,
        "type" => 2,
        "function" => 3,
        "enum_case" => 6,
        "property" => 5,
        _ => 0,
    }
}

#[no_mangle]
pub extern "C" fn basalt_lang() -> i32 {
    LANG_EXT.as_ptr() as i32
}

/// Declarative lexical profile for Layer 1 Phantom Safety Net delimiter validation.
static LEXICAL_PROFILE: &[u8] = b"{\
  \"version\": 1,\
  \"extensions\": [\"gleam\"],\
  \"line_comments\": [\"//\"],\
  \"block_comment\": null,\
  \"strings\": [\
    {\"open\": \"\\\"\", \"close\": \"\\\"\"}\
  ]\
}\0";

#[no_mangle]
pub extern "C" fn basalt_lexical_profile() -> i32 {
    LEXICAL_PROFILE.as_ptr() as i32
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
    src_ptr: i32,
    src_len: i32,
    out_ptr: i32,
    max_spans: i32,
) -> i32 {
    let src = std::slice::from_raw_parts(src_ptr as usize as *const u8, src_len as usize);
    let out =
        std::slice::from_raw_parts_mut(out_ptr as usize as *mut u8, (max_spans as usize) * 12);
    let state = match get_state() {
        Some(s) => s,
        None => return 0,
    };
    state.parser.reset();
    let Some(tree) = state.parser.parse(src, None) else {
        return 0;
    };

    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&state.parse_query, tree.root_node(), src);
    let mut count = 0usize;
    for m in matches {
        for cap in m.captures {
            if count >= max_spans as usize {
                break;
            }
            let Some(scope_name) = state.parse_cap_names.get(cap.index as usize) else {
                continue;
            };
            let scope_id = scope_id_for(scope_name);
            if scope_id == 0 {
                continue;
            }
            let offset = cap.node.start_byte() as u32;
            let length = (cap.node.end_byte() - cap.node.start_byte()) as u32;
            let base = count * 12;
            out[base..base + 4].copy_from_slice(&offset.to_le_bytes());
            out[base + 4..base + 8].copy_from_slice(&length.to_le_bytes());
            out[base + 8] = scope_id;
            out[base + 9] = 0;
            out[base + 10] = 0;
            out[base + 11] = 0;
            count += 1;
        }
    }
    count as i32
}

#[no_mangle]
pub unsafe extern "C" fn basalt_retrieval_chunks(
    src_ptr: i32,
    src_len: i32,
    out_ptr: i32,
    max_chunks: i32,
) -> i32 {
    let src = std::slice::from_raw_parts(src_ptr as usize as *const u8, src_len as usize);
    let out =
        std::slice::from_raw_parts_mut(out_ptr as usize as *mut u8, (max_chunks as usize) * 104);
    let state = match get_state() {
        Some(s) => s,
        None => return 0,
    };
    state.parser.reset();
    let Some(tree) = state.parser.parse(src, None) else {
        return 0;
    };

    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&state.retrieval_query, tree.root_node(), src);
    let mut count = 0usize;
    for m in matches {
        if count >= max_chunks as usize {
            break;
        }
        let mut offset = None::<u32>;
        let mut length = None::<u32>;
        let mut kind = None::<&str>;
        let mut name = None::<&str>;

        for cap in m.captures {
            let Some(cap_name) = state.retrieval_cap_names.get(cap.index as usize) else {
                continue;
            };
            if let Some(k) = cap_name.strip_prefix("chunk.") {
                offset = Some(cap.node.start_byte() as u32);
                length = Some((cap.node.end_byte() - cap.node.start_byte()) as u32);
                kind = Some(k);
            } else if cap_name.starts_with("name.") {
                if let Ok(text) = cap.node.utf8_text(src) {
                    name = Some(text.trim());
                }
            }
        }

        let (Some(off), Some(len), Some(k)) = (offset, length, kind) else {
            continue;
        };
        let label = if let Some(n) = name {
            let mut s = String::from(k);
            s.push(' ');
            s.push_str(n);
            s
        } else {
            k.to_string()
        };

        let base = count * 104;
        out[base..base + 4].copy_from_slice(&off.to_le_bytes());
        out[base + 4..base + 8].copy_from_slice(&len.to_le_bytes());
        let label_bytes = label.as_bytes();
        let label_len = label_bytes.len().min(95);
        out[base + 8..base + 8 + label_len].copy_from_slice(&label_bytes[..label_len]);
        out[base + 8 + label_len] = 0;
        out[base + 103] = kind_byte(k);
        count += 1;
    }
    count as i32
}

#[no_mangle]
pub unsafe extern "C" fn basalt_call_sites(
    src_ptr: i32,
    src_len: i32,
    out_ptr: i32,
    max_sites: i32,
) -> i32 {
    let src = std::slice::from_raw_parts(src_ptr as usize as *const u8, src_len as usize);
    let out =
        std::slice::from_raw_parts_mut(out_ptr as usize as *mut u8, (max_sites as usize) * 68);
    let state = match get_state() {
        Some(s) => s,
        None => return 0,
    };
    state.parser.reset();
    let Some(tree) = state.parser.parse(src, None) else {
        return 0;
    };

    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&state.call_sites_query, tree.root_node(), src);
    let mut count = 0usize;
    for m in matches {
        if count >= max_sites as usize {
            break;
        }
        for cap in m.captures {
            let Ok(name) = cap.node.utf8_text(src) else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let offset = cap.node.start_byte() as u32;
            let base = count * 68;
            out[base..base + 4].copy_from_slice(&offset.to_le_bytes());
            let name_bytes = name.as_bytes();
            let name_len = name_bytes.len().min(63);
            out[base + 4..base + 4 + name_len].copy_from_slice(&name_bytes[..name_len]);
            out[base + 4 + name_len] = 0;
            count += 1;
        }
    }
    count as i32
}

// ── Pure helpers used by unit tests ─────────────────────────────────────────

/// Parse `src` and return `(offset, length, label, kind)` retrieval chunks.
#[cfg_attr(not(test), allow(dead_code))]
fn retrieval_chunks(src: &[u8]) -> Vec<(u32, u32, String, u8)> {
    let Some(state) = make_state() else {
        return Vec::new();
    };
    let mut parser = state.parser;
    let Some(tree) = parser.parse(src, None) else {
        return Vec::new();
    };
    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&state.retrieval_query, tree.root_node(), src);
    let mut chunks = Vec::new();
    for m in matches {
        let mut offset = None;
        let mut length = None;
        let mut kind = None;
        let mut name = None;
        for cap in m.captures {
            let cap_name = &state.retrieval_cap_names[cap.index as usize];
            if let Some(k) = cap_name.strip_prefix("chunk.") {
                offset = Some(cap.node.start_byte() as u32);
                length = Some((cap.node.end_byte() - cap.node.start_byte()) as u32);
                kind = Some(k);
            } else if cap_name.starts_with("name.") {
                if let Ok(text) = cap.node.utf8_text(src) {
                    name = Some(text.trim().to_string());
                }
            }
        }
        if let (Some(off), Some(len), Some(k)) = (offset, length, kind) {
            let label = match name {
                Some(n) => format!("{k} {n}"),
                None => k.to_string(),
            };
            chunks.push((off, len, label, kind_byte(k)));
        }
    }
    chunks
}

/// Parse `src` and return `(offset, callee)` call sites.
#[cfg_attr(not(test), allow(dead_code))]
fn call_sites(src: &[u8]) -> Vec<(u32, String)> {
    let Some(state) = make_state() else {
        return Vec::new();
    };
    let mut parser = state.parser;
    let Some(tree) = parser.parse(src, None) else {
        return Vec::new();
    };
    let mut cursor = QueryCursor::new();
    let matches = cursor.matches(&state.call_sites_query, tree.root_node(), src);
    let mut sites = Vec::new();
    for m in matches {
        for cap in m.captures {
            if let Ok(text) = cap.node.utf8_text(src) {
                let text = text.trim();
                if !text.is_empty() {
                    sites.push((cap.node.start_byte() as u32, text.to_string()));
                }
            }
        }
    }
    sites
}

#[cfg(test)]
mod tests {
    use super::*;

    // Excerpt adapted from add-helper/src/add_helper/calendar.gleam.
    const CALENDAR: &str = include_str!("../tests/fixtures/calendar_sample.gleam");

    #[test]
    fn state_builds_queries() {
        assert!(make_state().is_some(), "tree-sitter queries must compile");
    }

    #[test]
    fn retrieval_finds_functions_types_and_imports() {
        let chunks = retrieval_chunks(CALENDAR.as_bytes());
        let labels: Vec<&str> = chunks.iter().map(|c| c.2.as_str()).collect();

        assert!(labels.iter().any(|l| *l == "function parse_query"));
        assert!(labels.iter().any(|l| *l == "function normalize"));
        assert!(labels.iter().any(|l| l.starts_with("type CalendarQuery")));
        assert!(labels.iter().any(|l| l.starts_with("type CalendarState")));
        assert!(labels.iter().any(|l| *l == "property default_query"));
        assert!(labels.iter().any(|l| *l == "module add_helper/models"));
        assert!(labels.iter().any(|l| *l == "enum_case Today"));
    }

    #[test]
    fn retrieval_reports_semantic_kinds() {
        let chunks = retrieval_chunks(CALENDAR.as_bytes());
        let kind_of = |label: &str| {
            chunks
                .iter()
                .find(|c| c.2 == label)
                .map(|c| c.3)
                .unwrap_or(0)
        };
        assert_eq!(kind_of("function parse_query"), 3);
        assert_eq!(kind_of("module add_helper/models"), 1);
        assert!(chunks
            .iter()
            .any(|c| c.2.starts_with("type CalendarQuery") && c.3 == 2));
    }

    #[test]
    fn call_sites_capture_bare_and_field_calls() {
        let src = br#"
pub fn go(events: List(Int)) -> Nil {
  let sorted = list.sort(events, int.compare)
  io.println("done")
  sorted
}
"#;
        let sites = call_sites(src);
        let names: Vec<&str> = sites.iter().map(|(_, n)| n.as_str()).collect();
        assert!(names.contains(&"sort"), "sites: {names:?}");
        assert!(names.contains(&"println"), "sites: {names:?}");
    }

    #[test]
    fn pipeline_right_hand_side_is_a_call_site() {
        let src = br#"
pub fn normalize(input: String) -> String {
  input |> string.trim |> string.lowercase
}
"#;
        let names: Vec<String> = call_sites(src).into_iter().map(|(_, n)| n).collect();
        assert!(names.contains(&"trim".to_string()), "sites: {names:?}");
        assert!(names.contains(&"lowercase".to_string()), "sites: {names:?}");
    }

    #[test]
    fn parse_spans_cover_keywords_and_strings() {
        let src = br#"pub fn main() { io.println("hello") }"#;
        let Some(state) = make_state() else {
            panic!("state");
        };
        let mut parser = state.parser;
        let tree = parser.parse(src, None).unwrap();
        let mut cursor = QueryCursor::new();
        let matches = cursor.matches(&state.parse_query, tree.root_node(), src.as_slice());
        let mut kinds = Vec::new();
        for m in matches {
            for cap in m.captures {
                let name = &state.parse_cap_names[cap.index as usize];
                kinds.push(scope_id_for(name));
            }
        }
        assert!(kinds.contains(&SCOPE_KEYWORD));
        assert!(kinds.contains(&SCOPE_STRING));
        assert!(kinds.contains(&SCOPE_FUNCTION));
    }

    /// Opt-in smoke test against a real add-helper source tree.
    #[test]
    #[ignore = "requires ../../add-helper"]
    fn parses_real_add_helper_modules() {
        for rel in [
            "../../add-helper/src/add_helper/calendar.gleam",
            "../../add-helper/src/add_helper/gemini.gleam",
            "../../add-helper/src/add_helper/server.gleam",
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
            let src = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
            let chunks = retrieval_chunks(&src);
            assert!(!chunks.is_empty(), "no retrieval chunks for {path:?}");
            assert!(
                chunks.iter().any(|c| c.3 == 3),
                "no function chunks in {path:?}"
            );
        }
    }
}

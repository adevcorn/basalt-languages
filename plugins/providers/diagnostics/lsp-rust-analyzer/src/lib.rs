//! LSP plugin: rust-analyzer
//!
//! # What this plugin does
//!
//! Tier-1 `CAP_DIAGNOSTICS | CAP_HOVER` plugin for Rust files.
//! Lazily spawns `rust-analyzer` on first use and communicates via JSON-RPC.
//!
//! # Build & install
//!
//! ```sh
//! cargo build --target wasm32-unknown-unknown --release
//! cp target/wasm32-unknown-unknown/release/lsp_rust_analyzer.wasm \
//!    ~/.config/basalt/plugins/lsp-rust-analyzer.wasm
//! ```

use basalt_plugin_sdk::prelude::*;

// ── Plugin identity & metadata ────────────────────────────────────────────────

basalt_plugin_meta! {
    name:         "lsp-rust-analyzer",
    version:      "0.1.0",
    hook_flags:   CAP_DIAGNOSTICS | CAP_HOVER,
    provides:     "diagnostics:rust",
    requires:     "project-model:cargo",
    file_globs:   "**/*.rs",
    activates_on: "**/Cargo.toml",
    activation_events: "",
}

// ── Host imports ──────────────────────────────────────────────────────────────

extern "C" {
    fn basalt_log(level: i32, msg_ptr: i32, msg_len: i32);
    fn basalt_spawn_lsp(cmd_ptr: i32, cmd_len: i32, root_ptr: i32, root_len: i32) -> i32;
    fn basalt_lsp_write(handle: i32, buf_ptr: i32, buf_len: i32) -> i32;
    fn basalt_lsp_read(handle: i32, out_ptr: i32, out_cap: i32) -> i32;
    fn basalt_lsp_stop(handle: i32);
}

// ── State ─────────────────────────────────────────────────────────────────────

static mut LSP_HANDLE: i32 = -1;
static mut SEQ_ID: u32 = 0;
static mut READ_BUF: [u8; 65536] = [0u8; 65536];
static mut ACCUM_LEN: usize = 0;
static mut ROOT_BUF: [u8; 1024] = [0u8; 1024];
static mut ROOT_LEN: usize = 0;

// ── Logging helpers ───────────────────────────────────────────────────────────

fn log_info(msg: &[u8]) {
    unsafe { basalt_log(2, msg.as_ptr() as i32, msg.len() as i32) }
}

fn log_err(msg: &[u8]) {
    unsafe { basalt_log(0, msg.as_ptr() as i32, msg.len() as i32) }
}

// ── LSP init ──────────────────────────────────────────────────────────────────

fn ensure_lsp(root: &[u8]) -> bool {
    unsafe {
        if LSP_HANDLE >= 0 {
            return true;
        }
        let rlen = root.len().min(ROOT_BUF.len() - 1);
        ROOT_BUF[..rlen].copy_from_slice(&root[..rlen]);
        ROOT_BUF[rlen] = 0;
        ROOT_LEN = rlen;

        let cmd = b"rust-analyzer\0";
        let handle = basalt_spawn_lsp(
            cmd.as_ptr() as i32,
            cmd.len() as i32,
            root.as_ptr() as i32,
            rlen as i32,
        );
        if handle < 0 {
            log_err(b"lsp-rust-analyzer: failed to spawn rust-analyzer");
            return false;
        }
        LSP_HANDLE = handle;
        send_initialize(handle, root);
        true
    }
}

// ── JSON-RPC helpers (hand-rolled) ────────────────────────────────────────────

static mut JSON_BUF: [u8; 32768] = [0u8; 32768];

unsafe fn jw(buf: &mut [u8], pos: usize, src: &[u8]) -> usize {
    let end = (pos + src.len()).min(buf.len());
    let n = end - pos;
    buf[pos..end].copy_from_slice(&src[..n]);
    end
}

unsafe fn jw_u32(buf: &mut [u8], mut pos: usize, mut v: u32) -> usize {
    if v == 0 {
        if pos < buf.len() {
            buf[pos] = b'0';
            pos += 1;
        }
        return pos;
    }
    let mut tmp = [0u8; 10];
    let mut tlen = 0usize;
    while v > 0 {
        tmp[tlen] = b'0' + (v % 10) as u8;
        tlen += 1;
        v /= 10;
    }
    for i in 0..tlen {
        if pos + i < buf.len() {
            buf[pos + i] = tmp[tlen - 1 - i];
        }
    }
    pos + tlen
}

unsafe fn jw_str(buf: &mut [u8], mut pos: usize, s: &[u8]) -> usize {
    if pos < buf.len() {
        buf[pos] = b'"';
        pos += 1;
    }
    for &b in s {
        match b {
            b'"' => {
                pos = jw(buf, pos, b"\\\"");
            }
            b'\\' => {
                pos = jw(buf, pos, b"\\\\");
            }
            b'\n' => {
                pos = jw(buf, pos, b"\\n");
            }
            b'\r' => {
                pos = jw(buf, pos, b"\\r");
            }
            b'\t' => {
                pos = jw(buf, pos, b"\\t");
            }
            _ => {
                if pos < buf.len() {
                    buf[pos] = b;
                    pos += 1;
                }
            }
        }
    }
    if pos < buf.len() {
        buf[pos] = b'"';
        pos += 1;
    }
    pos
}

unsafe fn build_framed(payload: &[u8]) -> usize {
    let body_len = payload.len();
    let mut hdr = [0u8; 64];
    let mut hp = 0usize;
    hp = jw(&mut hdr, hp, b"Content-Length: ");
    hp = jw_u32(&mut hdr, hp, body_len as u32);
    hp = jw(&mut hdr, hp, b"\r\n\r\n");
    let total = hp + body_len;
    if total <= JSON_BUF.len() {
        for i in (0..body_len).rev() {
            JSON_BUF[hp + i] = payload[i];
        }
        JSON_BUF[..hp].copy_from_slice(&hdr[..hp]);
    }
    total
}

unsafe fn send_notification(handle: i32, method: &[u8], params_json: &[u8]) {
    let mut body = [0u8; 8192];
    let mut p = 0usize;
    p = jw(&mut body, p, b"{\"jsonrpc\":\"2.0\",\"method\":");
    p = jw_str(&mut body, p, method);
    p = jw(&mut body, p, b",\"params\":");
    let plen = params_json.len().min(body.len() - p);
    body[p..p + plen].copy_from_slice(&params_json[..plen]);
    p += plen;
    p = jw(&mut body, p, b"}");
    let total = build_framed(&body[..p]);
    if total <= JSON_BUF.len() {
        basalt_lsp_write(handle, JSON_BUF.as_ptr() as i32, total as i32);
    }
}

unsafe fn send_request(handle: i32, method: &[u8], params_json: &[u8]) -> u32 {
    SEQ_ID += 1;
    let id = SEQ_ID;
    let mut body = [0u8; 8192];
    let mut p = 0usize;
    p = jw(&mut body, p, b"{\"jsonrpc\":\"2.0\",\"id\":");
    p = jw_u32(&mut body, p, id);
    p = jw(&mut body, p, b",\"method\":");
    p = jw_str(&mut body, p, method);
    p = jw(&mut body, p, b",\"params\":");
    let plen = params_json.len().min(body.len() - p);
    body[p..p + plen].copy_from_slice(&params_json[..plen]);
    p += plen;
    p = jw(&mut body, p, b"}");
    let total = build_framed(&body[..p]);
    if total <= JSON_BUF.len() {
        basalt_lsp_write(handle, JSON_BUF.as_ptr() as i32, total as i32);
    }
    id
}

// ── LSP initialization ────────────────────────────────────────────────────────

unsafe fn send_initialize(handle: i32, root: &[u8]) {
    let mut params = [0u8; 2048];
    let mut p = 0usize;
    p = jw(&mut params, p, b"{\"processId\":null,\"rootUri\":\"file://");
    let rlen = root.len().min(params.len() - p - 64);
    params[p..p + rlen].copy_from_slice(&root[..rlen]);
    p += rlen;
    p = jw(&mut params, p, b"\",\"capabilities\":{\"textDocument\":{\"hover\":{\"contentFormat\":[\"markdown\",\"plaintext\"]},\"publishDiagnostics\":{}}},\"clientInfo\":{\"name\":\"basalt\",\"version\":\"0.1.0\"}}");
    send_request(handle, b"initialize", &params[..p]);
    drain_until_response(handle, SEQ_ID, 3000);
    send_notification(handle, b"initialized", b"{}");
}

// ── Reading / parsing LSP stdout ──────────────────────────────────────────────

const POLL_DEADLINE_LOOPS: usize = 200;
const POLL_READ_LOOPS: usize = 50;

unsafe fn read_available(handle: i32) -> usize {
    for _ in 0..POLL_READ_LOOPS {
        let space = READ_BUF.len().saturating_sub(ACCUM_LEN);
        if space == 0 {
            break;
        }
        let n = basalt_lsp_read(
            handle,
            READ_BUF.as_ptr().add(ACCUM_LEN) as i32,
            space as i32,
        );
        if n <= 0 {
            break;
        }
        ACCUM_LEN += n as usize;
    }
    ACCUM_LEN
}

unsafe fn take_message_stashed() -> Option<usize> {
    let buf = &READ_BUF[..ACCUM_LEN];
    let cl_prefix = b"Content-Length: ";
    let cl_pos = find_bytes(buf, cl_prefix)?;
    let num_start = cl_pos + cl_prefix.len();
    let mut num_end = num_start;
    while num_end < buf.len() && buf[num_end].is_ascii_digit() {
        num_end += 1;
    }
    if num_end == num_start {
        return None;
    }
    let body_len = parse_decimal(&buf[num_start..num_end]);
    let sep = b"\r\n\r\n";
    let sep_pos = find_bytes(&buf[num_end..], sep)?;
    let body_start = num_end + sep_pos + sep.len();
    let body_end = body_start + body_len;
    if body_end > ACCUM_LEN {
        return None;
    }
    let copy_len = body_len.min(JSON_BUF.len());
    core::ptr::copy_nonoverlapping(
        READ_BUF.as_ptr().add(body_start),
        JSON_BUF.as_mut_ptr(),
        copy_len,
    );
    let remaining = ACCUM_LEN - body_end;
    core::ptr::copy(
        READ_BUF.as_ptr().add(body_end),
        READ_BUF.as_mut_ptr(),
        remaining,
    );
    ACCUM_LEN = remaining;
    Some(copy_len)
}

unsafe fn drain_until_response(handle: i32, req_id: u32, max_loops: usize) -> usize {
    for _ in 0..max_loops {
        read_available(handle);
        loop {
            match take_message_stashed() {
                None => break,
                Some(len) => {
                    let body = &JSON_BUF[..len];
                    if contains_id(body, req_id) && !is_notification(body) {
                        return len;
                    }
                }
            }
        }
    }
    0
}

unsafe fn drain_until_diagnostics(handle: i32, uri: &[u8], max_loops: usize) -> usize {
    let method = b"\"method\":\"textDocument/publishDiagnostics\"";
    for _ in 0..max_loops {
        read_available(handle);
        loop {
            match take_message_stashed() {
                None => break,
                Some(len) => {
                    let body = &JSON_BUF[..len];
                    if find_bytes(body, method).is_some() && find_bytes(body, uri).is_some() {
                        return len;
                    }
                }
            }
        }
    }
    0
}

// ── Minimal JSON helpers ──────────────────────────────────────────────────────

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn parse_decimal(s: &[u8]) -> usize {
    let mut v = 0usize;
    for &b in s {
        if b.is_ascii_digit() {
            v = v * 10 + (b - b'0') as usize;
        } else {
            break;
        }
    }
    v
}

fn is_notification(body: &[u8]) -> bool {
    find_bytes(body, b"\"method\"").is_some() && find_bytes(body, b"\"id\"").is_none()
}

fn contains_id(body: &[u8], id: u32) -> bool {
    let prefix = b"\"id\":";
    let mut pos = 0;
    while let Some(p) = find_bytes(&body[pos..], prefix) {
        let num_start = pos + p + prefix.len();
        let num_end = body[num_start..]
            .iter()
            .position(|&b| !b.is_ascii_digit())
            .map(|n| num_start + n)
            .unwrap_or(body.len());
        if parse_decimal(&body[num_start..num_end]) == id as usize {
            return true;
        }
        pos = num_start;
    }
    false
}

// ── LSP ↔ Basalt coordinate conversion ───────────────────────────────────────

unsafe fn byte_offset_to_lsp_pos(src: &[u8], byte_offset: usize, out: &mut [u8]) -> usize {
    let offset = byte_offset.min(src.len());
    let mut line: u32 = 0;
    let mut col_utf16: u32 = 0;
    let mut i = 0usize;
    while i < offset {
        if src[i] == b'\n' {
            line += 1;
            col_utf16 = 0;
            i += 1;
        } else {
            let b = src[i];
            let (codepoint, advance) = if b < 0x80 {
                (b as u32, 1)
            } else if b < 0xE0 {
                let c =
                    ((b as u32 & 0x1F) << 6) | (src.get(i + 1).copied().unwrap_or(0) as u32 & 0x3F);
                (c, 2)
            } else if b < 0xF0 {
                let c = ((b as u32 & 0x0F) << 12)
                    | ((src.get(i + 1).copied().unwrap_or(0) as u32 & 0x3F) << 6)
                    | (src.get(i + 2).copied().unwrap_or(0) as u32 & 0x3F);
                (c, 3)
            } else {
                let c = ((b as u32 & 0x07) << 18)
                    | ((src.get(i + 1).copied().unwrap_or(0) as u32 & 0x3F) << 12)
                    | ((src.get(i + 2).copied().unwrap_or(0) as u32 & 0x3F) << 6)
                    | (src.get(i + 3).copied().unwrap_or(0) as u32 & 0x3F);
                (c, 4)
            };
            col_utf16 += if codepoint >= 0x10000 { 2 } else { 1 };
            i += advance;
        }
    }
    let mut p = 0usize;
    p = unsafe { jw(out, p, b"{\"line\":") };
    p = unsafe { jw_u32(out, p, line) };
    p = unsafe { jw(out, p, b",\"character\":") };
    p = unsafe { jw_u32(out, p, col_utf16) };
    p = unsafe { jw(out, p, b"}") };
    p
}

fn lsp_pos_to_byte_offset(src: &[u8], line_target: u32, col_utf16_target: u32) -> usize {
    let mut line: u32 = 0;
    let mut col_utf16: u32 = 0;
    let mut i = 0usize;
    while i < src.len() {
        if line == line_target && col_utf16 == col_utf16_target {
            return i;
        }
        if src[i] == b'\n' {
            line += 1;
            col_utf16 = 0;
            i += 1;
        } else {
            let b = src[i];
            let (codepoint, advance) = if b < 0x80 {
                (b as u32, 1)
            } else if b < 0xE0 {
                let c =
                    ((b as u32 & 0x1F) << 6) | (src.get(i + 1).copied().unwrap_or(0) as u32 & 0x3F);
                (c, 2)
            } else if b < 0xF0 {
                let c = ((b as u32 & 0x0F) << 12)
                    | ((src.get(i + 1).copied().unwrap_or(0) as u32 & 0x3F) << 6)
                    | (src.get(i + 2).copied().unwrap_or(0) as u32 & 0x3F);
                (c, 3)
            } else {
                let c = ((b as u32 & 0x07) << 18)
                    | ((src.get(i + 1).copied().unwrap_or(0) as u32 & 0x3F) << 12)
                    | ((src.get(i + 2).copied().unwrap_or(0) as u32 & 0x3F) << 6)
                    | (src.get(i + 3).copied().unwrap_or(0) as u32 & 0x3F);
                (c, 4)
            };
            col_utf16 += if codepoint >= 0x10000 { 2 } else { 1 };
            i += advance;
        }
    }
    if line == line_target && col_utf16 == col_utf16_target {
        return i;
    }
    usize::MAX
}

unsafe fn build_uri(root: &[u8], path: &[u8], out: &mut [u8]) -> usize {
    let mut p = 0usize;
    p = jw(out, p, b"file://");
    let rlen = root.len().min(out.len() - p);
    out[p..p + rlen].copy_from_slice(&root[..rlen]);
    p += rlen;
    if p < out.len() && (root.is_empty() || root[root.len() - 1] != b'/') {
        out[p] = b'/';
        p += 1;
    }
    let plen = path.len().min(out.len() - p);
    out[p..p + plen].copy_from_slice(&path[..plen]);
    p += plen;
    p
}

// ── CAP_DIAGNOSTICS ───────────────────────────────────────────────────────────

const SEVERITY_ERROR: u8 = 0;
const SEVERITY_WARNING: u8 = 1;
const SEVERITY_INFO: u8 = 2;
const SEVERITY_HINT: u8 = 3;

/// Analyse `src` bytes and return diagnostics.
#[basalt_plugin]
fn diagnose(src: &[u8], path: &str) -> Vec<Diagnostic> {
    let path = path.as_bytes();
    unsafe {
        let root = &ROOT_BUF[..ROOT_LEN];
        if !ensure_lsp(root) {
            return Vec::new();
        }
        let handle = LSP_HANDLE;
        let mut uri_buf = [0u8; 2048];
        let uri_len = build_uri(root, path, &mut uri_buf);
        let uri = &uri_buf[..uri_len];
        send_did_open_or_change(handle, uri, src, path);
        let body_len = drain_until_diagnostics(handle, uri, POLL_DEADLINE_LOOPS);
        if body_len == 0 {
            return Vec::new();
        }
        let body = &JSON_BUF[..body_len];
        parse_publish_diagnostics_to_sdk(body, src)
    }
}

unsafe fn send_did_open_or_change(handle: i32, uri: &[u8], src: &[u8], _path: &[u8]) {
    let mut params = [0u8; 8192];
    let mut p = 0usize;
    p = jw(&mut params, p, b"{\"textDocument\":{\"uri\":");
    p = jw_str(&mut params, p, uri);
    p = jw(
        &mut params,
        p,
        b",\"languageId\":\"rust\",\"version\":1,\"text\":",
    );
    p = jw_str(&mut params, p, src);
    p = jw(&mut params, p, b"}}");
    send_notification(handle, b"textDocument/didOpen", &params[..p]);
}

/// Parse LSP `publishDiagnostics` JSON and return SDK [`Diagnostic`] values.
unsafe fn parse_publish_diagnostics_to_sdk(body: &[u8], src: &[u8]) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let mut search_pos = 0usize;

    let array_prefix = b"\"diagnostics\":[";
    let array_start = match find_bytes(body, array_prefix) {
        Some(p) => p + array_prefix.len(),
        None => return diags,
    };
    search_pos = array_start;

    loop {
        let obj_start = match find_bytes(&body[search_pos..], b"{") {
            Some(p) => search_pos + p,
            None => break,
        };
        let obj_end = match find_object_end(body, obj_start) {
            Some(e) => e,
            None => break,
        };
        let obj = &body[obj_start..obj_end];
        search_pos = obj_end;

        let severity: u8 = match extract_int_field(obj, b"\"severity\":") {
            Some(1) => SEVERITY_ERROR,
            Some(2) => SEVERITY_WARNING,
            Some(3) => SEVERITY_INFO,
            Some(4) => SEVERITY_HINT,
            _ => SEVERITY_ERROR,
        };

        let (start_line, start_char) = extract_range_start(obj);
        let (end_line, end_char) = extract_range_end(obj);

        let msg_start_idx = match extract_string_field_pos(obj, b"\"message\":") {
            Some(i) => i,
            None => continue,
        };
        let (msg_bytes, msg_len) = extract_string_bytes(obj, msg_start_idx);

        let byte_start = lsp_pos_to_byte_offset(src, start_line, start_char);
        let byte_end = lsp_pos_to_byte_offset(src, end_line, end_char);
        if byte_start == usize::MAX || byte_end == usize::MAX || byte_end < byte_start {
            continue;
        }
        let range_len = byte_end - byte_start;

        let sev = match severity {
            0 => basalt_plugin_sdk::Severity::Error,
            1 => basalt_plugin_sdk::Severity::Warning,
            2 => basalt_plugin_sdk::Severity::Info,
            _ => basalt_plugin_sdk::Severity::Hint,
        };
        let message = String::from_utf8_lossy(&msg_bytes[..msg_len]).into_owned();
        diags.push(Diagnostic::new(
            byte_start as u32,
            range_len as u32,
            message,
            sev,
        ));
    }

    diags
}

fn find_object_end(body: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escape = false;
    for (i, &b) in body[start..].iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if b == b'\\' && in_string {
            escape = true;
            continue;
        }
        if b == b'"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(start + i + 1);
            }
        }
    }
    None
}

fn extract_int_field(obj: &[u8], field: &[u8]) -> Option<i32> {
    let pos = find_bytes(obj, field)?;
    let num_start = pos + field.len();
    let mut v: i32 = 0;
    let mut found = false;
    for &b in &obj[num_start..] {
        if b.is_ascii_digit() {
            v = v * 10 + (b - b'0') as i32;
            found = true;
        } else if found {
            break;
        }
    }
    if found {
        Some(v)
    } else {
        None
    }
}

fn extract_range_start(obj: &[u8]) -> (u32, u32) {
    let prefix = b"\"start\":{\"line\":";
    let pos = match find_bytes(obj, prefix) {
        Some(p) => p + prefix.len(),
        None => return (0, 0),
    };
    let line = parse_decimal(&obj[pos..]) as u32;
    let char_prefix = b"\"character\":";
    let char_pos = match find_bytes(&obj[pos..], char_prefix) {
        Some(p) => pos + p + char_prefix.len(),
        None => return (line, 0),
    };
    (line, parse_decimal(&obj[char_pos..]) as u32)
}

fn extract_range_end(obj: &[u8]) -> (u32, u32) {
    let prefix = b"\"end\":{\"line\":";
    let pos = match find_bytes(obj, prefix) {
        Some(p) => p + prefix.len(),
        None => return (0, 0),
    };
    let line = parse_decimal(&obj[pos..]) as u32;
    let char_prefix = b"\"character\":";
    let char_pos = match find_bytes(&obj[pos..], char_prefix) {
        Some(p) => pos + p + char_prefix.len(),
        None => return (line, 0),
    };
    (line, parse_decimal(&obj[char_pos..]) as u32)
}

fn extract_string_field_pos(obj: &[u8], field: &[u8]) -> Option<usize> {
    let pos = find_bytes(obj, field)?;
    let after_field = pos + field.len();
    let quote_pos = obj[after_field..].iter().position(|&b| b == b'"')?;
    Some(after_field + quote_pos + 1)
}

fn extract_string_bytes(obj: &[u8], start: usize) -> (&[u8], usize) {
    let mut escape = false;
    for (i, &b) in obj[start..].iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if b == b'\\' {
            escape = true;
            continue;
        }
        if b == b'"' {
            return (&obj[start..start + i], i);
        }
    }
    (&obj[start..], 0)
}

// ── CAP_HOVER ─────────────────────────────────────────────────────────────────

#[basalt_plugin]
fn hover(src: &[u8], path: &str, byte_offset: u32) -> String {
    let path = path.as_bytes();
    unsafe {
        let root = &ROOT_BUF[..ROOT_LEN];
        if !ensure_lsp(root) {
            return String::new();
        }
        let handle = LSP_HANDLE;
        let mut uri_buf = [0u8; 2048];
        let uri_len = build_uri(root, path, &mut uri_buf);
        let uri = &uri_buf[..uri_len];
        send_did_open_or_change(handle, uri, src, path);

        let mut pos_buf = [0u8; 64];
        let pos_len = byte_offset_to_lsp_pos(src, byte_offset as usize, &mut pos_buf);
        let pos_json = &pos_buf[..pos_len];

        let mut params = [0u8; 4096];
        let mut p = 0usize;
        p = jw(&mut params, p, b"{\"textDocument\":{\"uri\":");
        p = jw_str(&mut params, p, uri);
        p = jw(&mut params, p, b"},\"position\":");
        let pjlen = pos_json.len().min(params.len() - p);
        params[p..p + pjlen].copy_from_slice(&pos_json[..pjlen]);
        p += pjlen;
        p = jw(&mut params, p, b"}");

        let req_id = send_request(handle, b"textDocument/hover", &params[..p]);
        let body_len = drain_until_response(handle, req_id, POLL_DEADLINE_LOOPS);
        if body_len == 0 {
            return String::new();
        }
        extract_hover_markdown_string(&JSON_BUF[..body_len])
    }
}

unsafe fn extract_hover_markdown_string(body: &[u8]) -> String {
    if find_bytes(body, b"\"result\":null").is_some() {
        return String::new();
    }
    let value_pos = match extract_string_field_pos(body, b"\"value\":") {
        Some(p) => p,
        None => match extract_string_field_pos(body, b"\"contents\":") {
            Some(p) => p,
            None => return String::new(),
        },
    };
    let (text, text_len) = extract_string_bytes(body, value_pos);
    if text_len == 0 {
        return String::new();
    }
    let mut out = Vec::with_capacity(text_len);
    out.resize(text_len, 0u8);
    let n = unescape_json_string(&text[..text_len], &mut out);
    out.truncate(n);
    String::from_utf8_lossy(&out).into_owned()
}

fn unescape_json_string(src: &[u8], dst: &mut [u8]) -> usize {
    let mut out = 0usize;
    let mut i = 0usize;
    while i < src.len() && out < dst.len() {
        if src[i] == b'\\' && i + 1 < src.len() {
            i += 1;
            let ch = match src[i] {
                b'n' => b'\n',
                b'r' => b'\r',
                b't' => b'\t',
                b'"' => b'"',
                b'\\' => b'\\',
                b'/' => b'/',
                other => other,
            };
            dst[out] = ch;
            out += 1;
        } else {
            dst[out] = src[i];
            out += 1;
        }
        i += 1;
    }
    out
}

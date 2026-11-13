//! rust-review-actions — Basalt plugin providing review-time actions for Rust workspaces.
//!
//! Provides: review-action:test:rust

use basalt_plugin_sdk::prelude::*;

const FILE_BUF_OFFSET: usize = 8 * 1024 * 1024;
const FILE_BUF_CAP: usize = 256 * 1024;

#[link(wasm_import_module = "env")]
extern "C" {
    fn basalt_read_file(path_ptr: i32, path_len: i32, out_ptr: i32, out_cap: i32) -> i32;
}

basalt_plugin_meta! {
    name:              "rust-review-actions",
    version:           env!("CARGO_PKG_VERSION"),
    hook_flags:        CAP_REVIEW_ACTIONS,
    provides:          "review-action:test:rust",
    requires:          "project-model:cargo",
    file_globs:        "",
    activates_on:      "",
    activation_events: "",
}

#[basalt_plugin]
fn review_actions(workspace_root: &str, _session_workspace: &str) -> Vec<ReviewActionDescriptor> {
    if !has_cargo_manifest(workspace_root) {
        return Vec::new();
    }

    vec![ReviewActionDescriptor {
        id: "cargo-test".into(),
        title: "Run Tests".into(),
        kind: ReviewActionKind::Test,
        ecosystem: "rust".into(),
        command_preview: "cargo test --color=never".into(),
        mutates_workspace: false,
        priority: 100,
    }]
}

#[basalt_plugin]
fn review_action_plan(
    action_id: &str,
    workspace_root: &str,
    _session_workspace: &str,
) -> Option<ReviewActionExecutionPlan> {
    if action_id != "cargo-test" || !has_cargo_manifest(workspace_root) {
        return None;
    }

    Some(ReviewActionExecutionPlan {
        executable: "cargo".into(),
        args: vec!["test".into(), "--color=never".into()],
        env: Vec::new(),
        cwd_mode: ReviewActionCwdMode::SessionWorkspace,
        output_category: "test".into(),
    })
}

#[derive(Default)]
struct ParseState {
    opened: bool,
}

impl ParseState {
    fn decode(bytes: &[u8]) -> Self {
        Self {
            opened: bytes.first().copied().unwrap_or(0) != 0,
        }
    }

    fn encode(&self) -> Vec<u8> {
        vec![self.opened as u8]
    }
}

#[basalt_plugin]
fn review_action_parse_line(action_id: &str, line: &[u8], state: &[u8]) -> (Vec<u8>, Vec<AgentEvent>) {
    if action_id != "cargo-test" {
        return (state.to_vec(), Vec::new());
    }
    let Ok(line_str) = std::str::from_utf8(line) else {
        return (state.to_vec(), Vec::new());
    };
    let trimmed = line_str.trim();
    if trimmed.is_empty() {
        return (state.to_vec(), Vec::new());
    }

    let mut parse_state = ParseState::decode(state);
    let mut events = Vec::new();
    if !parse_state.opened {
        parse_state.opened = true;
        events.push(AgentEvent::NewEntry {
            vendor_id: "cargo-test".into(),
            tool: "cargo test".into(),
            category: "test".into(),
            raw_cmd: "cargo test --color=never".into(),
            file_paths: vec![],
        });
    }

    if let Some(summary) = trimmed.strip_prefix("test result:") {
        events.push(AgentEvent::AppendToEntry {
            vendor_id: "cargo-test".into(),
            text: format!("test result:{}", summary),
        });
    }

    (parse_state.encode(), events)
}

fn has_cargo_manifest(root: &str) -> bool {
    let manifest = if root.ends_with('/') {
        format!("{root}Cargo.toml")
    } else {
        format!("{root}/Cargo.toml")
    };
    read_host_file(&manifest).is_some()
}

fn read_host_file(path: &str) -> Option<Vec<u8>> {
    let n = unsafe {
        basalt_read_file(
            path.as_ptr() as i32,
            path.len() as i32,
            FILE_BUF_OFFSET as i32,
            FILE_BUF_CAP as i32,
        )
    };
    if n <= 0 {
        return None;
    }
    let bytes = unsafe { std::slice::from_raw_parts(FILE_BUF_OFFSET as *const u8, n as usize) };
    Some(bytes.to_vec())
}

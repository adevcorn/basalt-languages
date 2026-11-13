//! Tier-1 Rust ECS Inspector Plugin for Bevy & ECS applications in Basalt.
//!
//! Analyzes Rust source files for Bevy systems, query access patterns,
//! mutable resource conflicts, and command usage.
//!
//! Dual-mode: compiled to WASM for the plugin registry (`diagnose` /
//! `hover` hooks via [`basalt_plugin_sdk`]), and linked natively into
//! `shell-bevy` as a Bevy district plugin (see `shell-bevy/src/plugins/`).
//! The native API below (`analyze_source`, metadata consts) is what the
//! Bevy shell uses — the WASM hooks delegate to the same analyzer.

use basalt_plugin_sdk::prelude::*;
// Force-link the single native definition of the SDK host imports (see
// ../../host-shims). Without a `use`, rustc prunes the rlib from final
// artifact links and Windows fails with LNK2019.
use syn::visit::{self, Visit};
use syn::{FnArg, ItemFn, Type};

basalt_plugin_meta! {
    name:         "rust-ecs-inspector",
    version:      "0.1.0",
    hook_flags:   CAP_DIAGNOSTICS | CAP_HOVER,
    provides:     "ecs-inspector@rust/v1",
    requires:     "project-model:cargo",
    optional_requires: "",
    file_globs:   "**/*.rs",
    activates_on: "",
    activation_events: "",
}

/// Identity + capability metadata shared by the WASM manifest and the
/// native Bevy district plugin (single source of truth).
pub const PLUGIN_NAME: &str = "rust-ecs-inspector";
pub const PLUGIN_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const PLUGIN_FILE_STEM: &str = "rust_ecs";
pub const PLUGIN_CAPABILITIES: &[&str] = &[
    "ECS_INSPECTOR",
    "CANVAS_ANNOTATION",
    "BEVY_SYSTEMS",
    "RESOURCE_CONFLICTS",
    "HOVER_CONTRACT",
];

/// True when an installed-plugin record refers to this plugin (WASM
/// sidecar stem match or capability match against the native list).
pub fn is_rust_ecs_plugin(name: &str, capabilities: &[String]) -> bool {
    let n = name.to_lowercase();
    n.contains("rust_ecs")
        || n.contains("rust-ecs")
        || n == "ecs"
        || n.contains("ecs_inspector")
    || capabilities.iter().any(|c| c == "ECS_INSPECTOR")
}

#[derive(Debug, Clone)]
pub struct EcsParam {
    pub kind: EcsParamKind,
    pub type_name: String,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EcsParamKind {
    Res,
    ResMut,
    Query,
    Commands,
    EventReader,
    EventWriter,
    NonSend,
    NonSendMut,
    Other,
}

#[derive(Debug, Clone)]
pub struct EcsSystemInfo {
    pub name: String,
    pub offset: u32,
    pub length: u32,
    pub params: Vec<EcsParam>,
    pub reads_resources: Vec<String>,
    pub writes_resources: Vec<String>,
    pub has_commands: bool,
    pub queries: Vec<String>,
}

struct EcsVisitor {
    systems: Vec<EcsSystemInfo>,
    source: String,
}

impl<'ast> Visit<'ast> for EcsVisitor {
    fn visit_item_fn(&mut self, item: &'ast ItemFn) {
        let fn_name = item.sig.ident.to_string();
        let mut ecs_params = Vec::new();
        let mut reads_res = Vec::new();
        let mut writes_res = Vec::new();
        let mut has_commands = false;
        let mut queries = Vec::new();

        for input in &item.sig.inputs {
            if let FnArg::Typed(pat_type) = input {
                let type_str = quote_type(&pat_type.ty);

                if type_str.contains("Commands") {
                    has_commands = true;
                    ecs_params.push(EcsParam {
                        kind: EcsParamKind::Commands,
                        type_name: "Commands".into(),
                        raw: type_str,
                    });
                } else if type_str.starts_with("Res <") || type_str.starts_with("Res<") {
                    let inner = extract_generic_inner(&type_str, "Res");
                    reads_res.push(inner.clone());
                    ecs_params.push(EcsParam {
                        kind: EcsParamKind::Res,
                        type_name: inner,
                        raw: type_str,
                    });
                } else if type_str.starts_with("ResMut <") || type_str.starts_with("ResMut<") {
                    let inner = extract_generic_inner(&type_str, "ResMut");
                    writes_res.push(inner.clone());
                    ecs_params.push(EcsParam {
                        kind: EcsParamKind::ResMut,
                        type_name: inner,
                        raw: type_str,
                    });
                } else if type_str.starts_with("Query <") || type_str.starts_with("Query<") {
                    let inner = extract_generic_inner(&type_str, "Query");
                    queries.push(inner.clone());
                    ecs_params.push(EcsParam {
                        kind: EcsParamKind::Query,
                        type_name: inner,
                        raw: type_str,
                    });
                } else if type_str.starts_with("EventReader <") || type_str.starts_with("EventReader<") {
                    let inner = extract_generic_inner(&type_str, "EventReader");
                    ecs_params.push(EcsParam {
                        kind: EcsParamKind::EventReader,
                        type_name: inner,
                        raw: type_str,
                    });
                } else if type_str.starts_with("EventWriter <") || type_str.starts_with("EventWriter<") {
                    let inner = extract_generic_inner(&type_str, "EventWriter");
                    ecs_params.push(EcsParam {
                        kind: EcsParamKind::EventWriter,
                        type_name: inner,
                        raw: type_str,
                    });
                }
            }
        }

        // Only treat as ECS system if it takes ECS parameters (Query, Res, ResMut, Commands, etc.)
        if !ecs_params.is_empty() {
            let (offset, length) = span_to_offset_len(&item.sig.ident, &self.source);
            self.systems.push(EcsSystemInfo {
                name: fn_name,
                offset,
                length,
                params: ecs_params,
                reads_resources: reads_res,
                writes_resources: writes_res,
                has_commands,
                queries,
            });
        }

        visit::visit_item_fn(self, item);
    }
}

fn quote_type(ty: &Type) -> String {
    let tokens = quote::quote!(#ty);
    tokens.to_string().replace("::", "::")
}

fn extract_generic_inner(type_str: &str, wrapper: &str) -> String {
    let clean = type_str.replace(" ", "");
    if let Some(start) = clean.find('<') {
        if let Some(end) = clean.rfind('>') {
            if end > start {
                return clean[start + 1..end].trim().to_string();
            }
        }
    }
    wrapper.to_string()
}

fn span_to_offset_len(ident: &syn::Ident, source: &str) -> (u32, u32) {
    let name = ident.to_string();
    if let Some(idx) = source.find(&format!("fn {}", name)) {
        let fn_ident_offset = idx + 3;
        (fn_ident_offset as u32, name.len() as u32)
    } else if let Some(idx) = source.find(&name) {
        (idx as u32, name.len() as u32)
    } else {
        (0, name.len() as u32)
    }
}

/// Native entry point: parse `src` with `syn` and return every Bevy ECS
/// system found. Used by `shell-bevy`'s district plugin and by the WASM
/// `diagnose` / `hover` hooks below (single analyzer for both hosts).
pub fn analyze_file(src: &str) -> Vec<EcsSystemInfo> {
    let Ok(file) = syn::parse_file(src) else {
        return Vec::new();
    };

    let mut visitor = EcsVisitor {
        systems: Vec::new(),
        source: src.to_string(),
    };
    visitor.visit_file(&file);
    visitor.systems
}

/// Alias kept for readability at native call sites.
pub fn analyze_source(src: &str) -> Vec<EcsSystemInfo> {
    analyze_file(src)
}

/// 1-based line number of `offset` within `src` (native helper for
/// district cards; mirrors the Bevy shell's slab line computation).
pub fn line_for_offset(src: &str, offset: u32) -> usize {
    src[..(offset as usize).min(src.len())]
        .lines()
        .count()
        .max(1)
}

#[basalt_plugin]
fn diagnose(src: &[u8], _path: &str) -> Vec<Diagnostic> {
    let Ok(content) = std::str::from_utf8(src) else {
        return Vec::new();
    };

    let systems = analyze_file(content);
    let mut diagnostics = Vec::new();

    // Check for potential ECS concurrency hazards within the file
    let mut res_mut_counts: std::collections::HashMap<String, Vec<&EcsSystemInfo>> = std::collections::HashMap::new();
    for sys in &systems {
        for w in &sys.writes_resources {
            res_mut_counts.entry(w.clone()).or_default().push(sys);
        }
    }

    for (res_name, writers) in res_mut_counts {
        if writers.len() > 1 {
            for sys in writers {
                diagnostics.push(Diagnostic::new(
                    sys.offset,
                    sys.length,
                    format!(
                        "⚡ [ECS Concurrency Alert] Multiple systems in this module mutate ResMut<{}>. Ensure explicit schedule ordering with .after() or .chain() to avoid ambiguous frame ordering.",
                        res_name
                    ),
                    Severity::Info,
                ));
            }
        }
    }

    // Diagnostics for unconstrained queries (mutable query with no filters)
    for sys in &systems {
        for q in &sys.queries {
            if q.contains("&mut") && !q.contains("With<") && !q.contains("Without<") && !q.contains("Added<") && !q.contains("Changed<") {
                diagnostics.push(Diagnostic::new(
                    sys.offset,
                    sys.length,
                    format!(
                        "⚡ [ECS Query Inspection] System '{}' uses broad mutable query '{}'. Consider narrowing scope with With<...> or Changed<...> filters.",
                        sys.name, q
                    ),
                    Severity::Hint,
                ));
            }
        }
    }

    diagnostics
}

#[basalt_plugin]
fn hover(src: &[u8], _path: &str, byte_offset: u32) -> String {
    let Ok(content) = std::str::from_utf8(src) else {
        return String::new();
    };

    let systems = analyze_file(content);

    for sys in systems {
        // Range around system name or signature
        if byte_offset >= sys.offset.saturating_sub(10) && byte_offset <= sys.offset + sys.length + 30 {
            let mut doc = format!("### ⚡ Bevy ECS System: {}\n\n", sys.name);
            doc.push_str("**ECS Query Contract & Execution Signature**:\n");

            if !sys.reads_resources.is_empty() {
                doc.push_str("- 📥 **Reads Resources**: ");
                let list = sys.reads_resources.iter().map(|r| format!("Res<{}>", r)).collect::<Vec<_>>().join(", ");
                doc.push_str(&list);
                doc.push('\n');
            }

            if !sys.writes_resources.is_empty() {
                doc.push_str("- ✏️ **Mutates Resources**: ");
                let list = sys.writes_resources.iter().map(|r| format!("ResMut<{}>", r)).collect::<Vec<_>>().join(", ");
                doc.push_str(&list);
                doc.push('\n');
            }

            if !sys.queries.is_empty() {
                doc.push_str("- 🔍 **Component Queries**:\n");
                for q in &sys.queries {
                    doc.push_str(&format!("  - Query<{}>\n", q));
                }
            }

            if sys.has_commands {
                doc.push_str("- 🏗️ **Entity Commands**: Spawns, despawns, or inserts components into entities via Commands\n");
            }

            return doc;
        }
    }

    String::new()
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct DomainFunctionEntry {
    pub name: String,
    pub line: usize,
    pub role: String,
    pub is_system: bool,
}

pub fn extract_domain_entry_points(src: &str) -> Vec<DomainFunctionEntry> {
    let systems = analyze_file(src);
    let mut entries = Vec::new();

    for sys in systems {
        let line = src[..sys.offset as usize].lines().count().max(1);
        entries.push(DomainFunctionEntry {
            name: sys.name,
            line,
            role: "Bevy ECS System".into(),
            is_system: true,
        });
    }

    entries
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub enum AnnotationCategory {
    Alert,
    Framework,
    DataStore,
    Info,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub enum AnnotationTarget {
    File,
    Function { name: String, line: usize },
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct CanvasAnnotation {
    pub target: AnnotationTarget,
    pub plugin_id: String,
    pub badge: String,
    pub category: AnnotationCategory,
    pub priority: u8,
    pub detail: Option<String>,
}

pub fn extract_annotations(src: &str, path: &str) -> Vec<CanvasAnnotation> {
    let systems = analyze_file(src);
    if systems.is_empty() {
        return Vec::new();
    }

    let mut annotations = Vec::new();

    // 1. File-level badge
    annotations.push(CanvasAnnotation {
        target: AnnotationTarget::File,
        plugin_id: "rust-ecs".into(),
        badge: format!("⚡ {} Systems", systems.len()),
        category: AnnotationCategory::Framework,
        priority: 50,
        detail: Some(format!("Contains {} Bevy ECS systems in {}", systems.len(), path)),
    });

    // Detect mutation conflicts
    let mut res_mut_counts: std::collections::HashMap<String, Vec<&EcsSystemInfo>> = std::collections::HashMap::new();
    for sys in &systems {
        for w in &sys.writes_resources {
            res_mut_counts.entry(w.clone()).or_default().push(sys);
        }
    }

    // 2. Function-level badges
    for sys in &systems {
        let line = src[..sys.offset as usize].lines().count().max(1);

        // Check if this system participates in a conflict
        let mut has_conflict = false;
        for w in &sys.writes_resources {
            if let Some(writers) = res_mut_counts.get(w) {
                if writers.len() > 1 {
                    has_conflict = true;
                    annotations.push(CanvasAnnotation {
                        target: AnnotationTarget::Function {
                            name: sys.name.clone(),
                            line,
                        },
                        plugin_id: "rust-ecs".into(),
                        badge: format!("⚠️ Race: ResMut<{}>", w),
                        category: AnnotationCategory::Alert,
                        priority: 95,
                        detail: Some(format!(
                            "Multiple systems in this module mutate ResMut<{}> concurrently without schedule ordering constraints.",
                            w
                        )),
                    });
                }
            }
        }

        // Primary signature badge (e.g. ResMut or Res or Queries)
        let primary_badge = if !sys.writes_resources.is_empty() {
            format!("ResMut<{}>", sys.writes_resources[0])
        } else if !sys.reads_resources.is_empty() {
            format!("Res<{}>", sys.reads_resources[0])
        } else if !sys.queries.is_empty() {
            "⚡ Query".into()
        } else if sys.has_commands {
            "⚡ Commands".into()
        } else {
            "⚡ ECS System".into()
        };

        let mut detail = format!("### ⚡ ECS Contract for `fn {}()`\n\n", sys.name);
        if !sys.reads_resources.is_empty() {
            detail.push_str(&format!("- **Reads**: {}\n", sys.reads_resources.join(", ")));
        }
        if !sys.writes_resources.is_empty() {
            detail.push_str(&format!("- **Mutates**: {}\n", sys.writes_resources.join(", ")));
        }
        if !sys.queries.is_empty() {
            detail.push_str(&format!("- **Queries**: {}\n", sys.queries.join(", ")));
        }
        if sys.has_commands {
            detail.push_str("- **Commands**: Spawns/despawns entities or components\n");
        }

        annotations.push(CanvasAnnotation {
            target: AnnotationTarget::Function {
                name: sys.name.clone(),
                line,
            },
            plugin_id: "rust-ecs".into(),
            badge: primary_badge,
            category: if has_conflict { AnnotationCategory::Alert } else { AnnotationCategory::Framework },
            priority: 60,
            detail: Some(detail),
        });
    }

    annotations
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bevy_systems() {
        let code = r#"
            fn camera_controller_system(
                mut query: Query<&mut Transform, With<MainCamera>>,
                mut camera_state: ResMut<CameraState>,
                input: Res<ButtonInput<MouseButton>>,
            ) {
            }

            fn plain_helper_fn(x: i32) -> i32 {
                x + 1
            }
        "#;

        let systems = analyze_file(code);
        assert_eq!(systems.len(), 1);
        let sys = &systems[0];
        assert_eq!(sys.name, "camera_controller_system");
        assert_eq!(sys.writes_resources, vec!["CameraState"]);
        assert_eq!(sys.reads_resources, vec!["ButtonInput<MouseButton>"]);
        assert_eq!(sys.queries.len(), 1);

        let annos = extract_annotations(code, "test.rs");
        assert_eq!(annos.len(), 2); // 1 file badge + 1 function badge
    }

    #[test]
    fn test_native_helpers() {
        assert!(is_rust_ecs_plugin("rust_ecs.wasm", &[]));
        assert!(is_rust_ecs_plugin("anything", &[String::from("ECS_INSPECTOR")]));
        assert!(!is_rust_ecs_plugin("project_model", &[]));
        assert_eq!(PLUGIN_CAPABILITIES.len(), 5);
        let code = "fn sys(a: Res<Foo>) {}\nfn plain(x: i32) -> i32 { x }";
        let systems = analyze_source(code);
        assert_eq!(systems.len(), 1);
        assert_eq!(line_for_offset(code, 0), 1);
    }
}


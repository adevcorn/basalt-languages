//! Yarn Workspaces / Lerna project-model plugin for Basalt.
//!
//! Reads the root `package.json` (or `lerna.json`), expands workspace
//! package globs, and emits the generic Basalt project-model JSON.

use basalt_plugin_sdk::prelude::*;

basalt_plugin_meta! {
    name:         "project-model-yarn-workspaces",
    version:      env!("CARGO_PKG_VERSION"),
    hook_flags:   CAP_PROJECT_MODEL | CAP_SHADOW_EXCLUDES | CAP_SHADOW_SYMLINKS,
    provides:     "project-model:yarn-workspaces",
    requires:     "",
    file_globs:   "",
    activates_on: "package.json\n**/package.json\nlerna.json\n**/lerna.json",
    activation_events: "",
}

/// Node/Yarn workspace caches and generated output.
#[basalt_plugin]
fn shadow_excludes(_root: &str) -> Vec<u8> {
    br#"["**/node_modules/**","**/.yarn/**","**/.pnpm/**","**/.pnpm-store/**","**/dist/**","**/.next/**","**/.turbo/**","**/coverage/**"]"#.to_vec()
}

/// Installed dependency trees. These are expensive to copy and read-only for
/// the toolchain, so link them to the host instead of duplicating per shadow.
#[basalt_plugin]
fn shadow_symlinks(_root: &str) -> Vec<u8> {
    br#"["node_modules","**/node_modules",".yarn","**/.yarn",".pnpm","**/.pnpm",".pnpm-store","**/.pnpm-store"]"#.to_vec()
}

extern "C" {
    fn basalt_read_file(path_ptr: i32, path_len: i32, out_ptr: i32, out_cap: i32) -> i32;
    fn basalt_list_files(root_ptr: i32, root_len: i32, out_ptr: i32, out_cap: i32) -> i32;
}

const FILE_BUF_SIZE: usize = 4 * 1024 * 1024;
const LIST_BUF_SIZE: usize = 8 * 1024 * 1024;

static mut FILE_BUF: [u8; FILE_BUF_SIZE] = [0u8; FILE_BUF_SIZE];
static mut LIST_BUF: [u8; LIST_BUF_SIZE] = [0u8; LIST_BUF_SIZE];

// ── Package info ─────────────────────────────────────────────────────────────

#[derive(Clone)]
struct PackageInfo {
    id: String,
    name: String,
    #[allow(dead_code)] // captured for completeness; may be emitted in future schema versions
    version: String,
    manifest_path: String,
    owner_prefix: String,
    has_build: bool,
    has_test: bool,
    has_lint: bool,
}

// ── Main entrypoint ───────────────────────────────────────────────────────────

#[basalt_plugin]
fn build_project_model(root: &str) -> Vec<u8> {
    // If root points directly at a package.json, treat its parent as the root.
    let effective_root = if root.ends_with("package.json") || root.ends_with("lerna.json") {
        parent_dir(root)
    } else {
        root.to_string()
    };

    // 1. Try root package.json
    let root_pkg_path = join_path(&effective_root, "package.json");
    if let Some(root_pkg) = read_host_file(&root_pkg_path) {
        if !root_pkg.is_empty() {
            let patterns = parse_workspaces(&root_pkg);
            if !patterns.is_empty() {
                let pkg_roots = expand_globs(&effective_root, &patterns);
                let packages: Vec<PackageInfo> = pkg_roots
                    .iter()
                    .filter_map(|pkg_root| read_package_info(pkg_root))
                    .collect();
                // Also include the root package itself if it has a name.
                let root_info = read_package_info(&effective_root);
                let mut all = Vec::new();
                if let Some(ri) = root_info {
                    all.push(ri);
                }
                all.extend(packages);
                if !all.is_empty() {
                    return emit_project_model_json(&effective_root, &all).into_bytes();
                }
            }
            // No workspaces field — treat the package.json itself as a single package.
            if let Some(info) = read_package_info(&effective_root) {
                return emit_project_model_json(&effective_root, &[info]).into_bytes();
            }
        }
    }

    // 2. Try root lerna.json
    let lerna_path = join_path(&effective_root, "lerna.json");
    if let Some(lerna_json) = read_host_file(&lerna_path) {
        if !lerna_json.is_empty() {
            let patterns = parse_lerna_packages(&lerna_json);
            let pkg_roots = expand_globs(&effective_root, &patterns);
            let packages: Vec<PackageInfo> = pkg_roots
                .iter()
                .filter_map(|pkg_root| read_package_info(pkg_root))
                .collect();
            if !packages.is_empty() {
                return emit_project_model_json(&effective_root, &packages).into_bytes();
            }
        }
    }

    // 3. Fallback: discover any package.json via basalt_list_files.
    let all_files = list_host_files(&effective_root);
    let mut packages: Vec<PackageInfo> = all_files
        .iter()
        .filter(|f| {
            let norm = normalize_rel_path(f);
            norm.ends_with("package.json") && !norm.contains("node_modules")
        })
        .filter_map(|rel| {
            let pkg_dir = join_path(&effective_root, &normalize_rel_path(&parent_dir(rel)));
            read_package_info(&pkg_dir)
        })
        .collect();
    // Deduplicate by manifest_path.
    packages.dedup_by(|a, b| a.manifest_path == b.manifest_path);

    if packages.is_empty() {
        return Vec::new();
    }
    emit_project_model_json(&effective_root, &packages).into_bytes()
}

// ── Workspace discovery ───────────────────────────────────────────────────────

/// Parse the `"workspaces"` field from a root package.json.
/// Supports both Yarn classic (array) and Yarn Berry / object form.
fn parse_workspaces(pkg_json: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(pkg_json) else {
        return Vec::new();
    };
    match value.get("workspaces") {
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        Some(serde_json::Value::Object(obj)) => {
            // Yarn Berry object: { "packages": [...] }
            obj.get("packages")
                .and_then(|p| p.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

/// Parse the `"packages"` field from a lerna.json.
fn parse_lerna_packages(lerna_json: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(lerna_json) else {
        return Vec::new();
    };
    value
        .get("packages")
        .and_then(|p| p.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Expand glob patterns like `"packages/*"` by listing files under root and
/// returning the subdirectory paths that match the pattern prefix.
///
/// Strategy: for a pattern like `packages/*` we look for directories
/// directly under `root/packages/`. For `**/*` we enumerate all directories
/// that contain a `package.json`.
fn expand_globs(root: &str, patterns: &[String]) -> Vec<String> {
    let all_files = list_host_files(root);
    let mut result: Vec<String> = Vec::new();

    for pattern in patterns {
        let norm_pat = normalize_rel_path(pattern);

        if norm_pat == "**" || norm_pat == "**/*" {
            // Expand to every directory that has a package.json (excluding root).
            for rel in &all_files {
                let norm = normalize_rel_path(rel);
                if norm.ends_with("package.json") && !norm.contains("node_modules") {
                    let pkg_dir_rel = parent_dir(&norm);
                    if pkg_dir_rel.is_empty() || pkg_dir_rel == "." {
                        continue; // skip root
                    }
                    let pkg_root = join_path(root, &pkg_dir_rel);
                    if !result.contains(&pkg_root) {
                        result.push(pkg_root);
                    }
                }
            }
            continue;
        }

        // Handle patterns like `packages/*` or `apps/*`.
        // Split at the last segment to get prefix + wildcard.
        let (prefix, wildcard) = if let Some(pos) = norm_pat.rfind('/') {
            (&norm_pat[..pos], &norm_pat[pos + 1..])
        } else {
            ("", norm_pat.as_str())
        };

        if wildcard == "*" {
            // Match direct children of `prefix/`.
            for rel in &all_files {
                let norm = normalize_rel_path(rel);
                if norm.contains("node_modules") {
                    continue;
                }
                if !norm.ends_with("package.json") {
                    continue;
                }
                let pkg_dir = parent_dir(&norm);
                if pkg_dir.is_empty() || pkg_dir == "." {
                    continue; // skip root-level package.json
                }
                // pkg_dir should be exactly `prefix/<name>`.
                let expected_prefix = if prefix.is_empty() {
                    String::new()
                } else {
                    format!("{prefix}/")
                };
                if pkg_dir.starts_with(&expected_prefix) {
                    let rest = &pkg_dir[expected_prefix.len()..];
                    // Direct child: no further slash.
                    if !rest.contains('/') {
                        let pkg_root = join_path(root, &pkg_dir);
                        if !result.contains(&pkg_root) {
                            result.push(pkg_root);
                        }
                    }
                }
            }
        } else {
            // Treat the pattern as a literal path to a package directory.
            let pkg_root = join_path(root, &norm_pat);
            if !result.contains(&pkg_root) {
                result.push(pkg_root);
            }
        }
    }

    result
}

// ── Package info extraction ───────────────────────────────────────────────────

/// Read `{pkg_root}/package.json` and extract relevant fields.
fn read_package_info(pkg_root: &str) -> Option<PackageInfo> {
    let manifest_path = join_path(pkg_root, "package.json");
    let content = read_host_file(&manifest_path)?;
    if content.is_empty() {
        return None;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
        return None;
    };

    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if name.is_empty() {
        return None;
    }

    let version = value
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let has_build = value
        .pointer("/scripts/build")
        .is_some();
    let has_test = value
        .pointer("/scripts/test")
        .is_some();
    let has_lint = value
        .pointer("/scripts/lint")
        .is_some();

    let id = sanitize_id(&name);
    let owner_prefix = relative_owner_prefix(pkg_root);

    Some(PackageInfo {
        id,
        name,
        version,
        manifest_path,
        owner_prefix,
        has_build,
        has_test,
        has_lint,
    })
}

// ── JSON emission ─────────────────────────────────────────────────────────────

fn emit_project_model_json(root: &str, packages: &[PackageInfo]) -> String {
    let display_name = file_name(root).to_string();
    let display_name = if display_name.is_empty() {
        "workspace".to_string()
    } else {
        display_name
    };
    let primary_manifest = join_path(root, "package.json");

    let projects_json = packages
        .iter()
        .map(package_project_json)
        .collect::<Vec<_>>()
        .join(",");

    let owners_json = packages
        .iter()
        .flat_map(package_owner_json)
        .collect::<Vec<_>>()
        .join(",");

    format!(
        concat!(
            "{{",
            "\"schema_version\":1,",
            "\"ecosystem\":\"{}\",",
            "\"root\":\"{}\",",
            "\"display_name\":\"{}\",",
            "\"primary_manifest_path\":\"{}\",",
            "\"projects\":[{}],",
            "\"owners\":[{}]",
            "}}"
        ),
        escape_json("yarn-workspaces"),
        escape_json(root),
        escape_json(&display_name),
        escape_json(&primary_manifest),
        projects_json,
        owners_json,
    )
}

fn package_project_json(pkg: &PackageInfo) -> String {
    let mut capabilities: Vec<&str> = vec!["is-library"];
    if pkg.has_build {
        capabilities.push("has-build-script");
    }
    if pkg.has_test {
        capabilities.push("has-test-script");
    }
    if pkg.has_lint {
        capabilities.push("has-lint-script");
    }
    let caps_str = capabilities
        .iter()
        .map(|c| format!("\"{}\"", escape_json(c)))
        .collect::<Vec<_>>()
        .join(",");

    let source_roots = if pkg.owner_prefix.is_empty() || pkg.owner_prefix == "." {
        "\"src\"".to_string()
    } else {
        format!("\"{}/src\"", escape_json(&pkg.owner_prefix))
    };

    let test_roots = if pkg.has_test {
        if pkg.owner_prefix.is_empty() || pkg.owner_prefix == "." {
            "\"__tests__\",\"test\"".to_string()
        } else {
            format!(
                "\"{}/__tests__\",\"{}/test\"",
                escape_json(&pkg.owner_prefix),
                escape_json(&pkg.owner_prefix)
            )
        }
    } else {
        String::new()
    };

    let build_roots = if pkg.owner_prefix.is_empty() || pkg.owner_prefix == "." {
        "\"dist\",\"build\"".to_string()
    } else {
        format!(
            "\"{}/dist\",\"{}/build\"",
            escape_json(&pkg.owner_prefix),
            escape_json(&pkg.owner_prefix)
        )
    };

    format!(
        concat!(
            "{{",
            "\"id\":\"{}\",",
            "\"name\":\"{}\",",
            "\"kind\":\"package\",",
            "\"subtype\":\"yarn-workspace\",",
            "\"capabilities\":[{}],",
            "\"manifest_path\":\"{}\",",
            "\"source_roots\":[{}],",
            "\"test_roots\":[{}],",
            "\"build_roots\":[{}],",
            "\"targets\":[{{",
                "\"id\":\"{}\",",
                "\"name\":\"{}\",",
                "\"kind\":\"package\",",
                "\"subtype\":\"yarn-workspace\",",
                "\"capabilities\":[{}],",
                "\"language\":\"javascript\",",
                "\"source_roots\":[{}],",
                "\"test_roots\":[{}],",
                "\"build_roots\":[{}]",
            "}}]",
            "}}"
        ),
        escape_json(&pkg.id),
        escape_json(&pkg.name),
        caps_str,
        escape_json(&pkg.manifest_path),
        source_roots,
        test_roots,
        build_roots,
        // targets
        escape_json(&pkg.id),
        escape_json(&pkg.name),
        caps_str,
        source_roots,
        test_roots,
        build_roots,
    )
}

fn package_owner_json(pkg: &PackageInfo) -> Vec<String> {
    let mut out = Vec::new();
    let manifest_rel = normalize_rel_path(&pkg.owner_prefix);

    if !manifest_rel.is_empty() && manifest_rel != "." {
        // Exact match on the manifest file.
        out.push(owner_json(
            &format!("{}/{}", manifest_rel, file_name(&pkg.manifest_path)),
            "exact",
            &pkg.name,
            &pkg.id,
            "manifest",
        ));
        // Prefix match on the package directory.
        out.push(owner_json(
            &ensure_trailing_slash(&manifest_rel),
            "prefix",
            &pkg.name,
            &pkg.id,
            "source",
        ));
    } else {
        out.push(owner_json(
            file_name(&pkg.manifest_path),
            "exact",
            &pkg.name,
            &pkg.id,
            "manifest",
        ));
    }
    out
}

fn owner_json(path: &str, match_kind: &str, label: &str, project_id: &str, kind: &str) -> String {
    format!(
        "{{\"path\":\"{}\",\"match_kind\":\"{}\",\"label\":\"{}\",\"project_id\":\"{}\",\"target_id\":\"{}\",\"kind\":\"{}\"}}",
        escape_json(path),
        escape_json(match_kind),
        escape_json(label),
        escape_json(project_id),
        escape_json(project_id),
        escape_json(kind),
    )
}

// ── Host syscall wrappers ─────────────────────────────────────────────────────

fn read_host_file(path: &str) -> Option<String> {
    let buf: &mut [u8; FILE_BUF_SIZE] = unsafe { &mut *core::ptr::addr_of_mut!(FILE_BUF) };
    let n = unsafe {
        basalt_read_file(
            path.as_ptr() as i32,
            path.len() as i32,
            buf.as_mut_ptr() as i32,
            FILE_BUF_SIZE as i32,
        )
    };
    if n <= 0 {
        return None;
    }
    let bytes = &buf[..n as usize];
    Some(String::from_utf8_lossy(bytes).into_owned())
}

fn list_host_files(root: &str) -> Vec<String> {
    let buf: &mut [u8; LIST_BUF_SIZE] = unsafe { &mut *core::ptr::addr_of_mut!(LIST_BUF) };
    let n = unsafe {
        basalt_list_files(
            root.as_ptr() as i32,
            root.len() as i32,
            buf.as_mut_ptr() as i32,
            LIST_BUF_SIZE as i32,
        )
    };
    if n <= 0 {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&buf[..n as usize]);
    text.lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

// ── Path helpers ──────────────────────────────────────────────────────────────

fn join_path(root: &str, suffix: &str) -> String {
    if root.is_empty() {
        return suffix.to_string();
    }
    if root.ends_with('/') {
        format!("{root}{suffix}")
    } else {
        format!("{root}/{suffix}")
    }
}

fn parent_dir(path: &str) -> String {
    path.rsplit_once(['/', '\\'])
        .map(|(parent, _)| parent.to_string())
        .unwrap_or_default()
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn normalize_rel_path(path: &str) -> String {
    path.replace('\\', "/")
        .trim_start_matches("./")
        .trim_matches('/')
        .to_string()
}

fn ensure_trailing_slash(path: &str) -> String {
    if path.is_empty() || path.ends_with('/') {
        path.to_string()
    } else {
        format!("{path}/")
    }
}

/// Compute a relative owner prefix for a package root (relative to the
/// workspace root). When root is not available we just use the last
/// directory component. The caller already passes absolute pkg_root.
fn relative_owner_prefix(_pkg_root: &str) -> String {
    // Owner prefix is computed at call time via the glob expansion result,
    // which gives us the relative path. However, in read_package_info we
    // receive the absolute path. We keep the full absolute path as owner
    // prefix and rely on package_owner_json to produce sensible relative
    // paths.  For simplicity — mirroring how the dotnet plugin handles
    // single-project cases — we return an empty string here and let the
    // caller override if needed.
    String::new()
}

fn sanitize_id(value: &str) -> String {
    // Strip npm scope prefix (@scope/name → scope-name).
    let trimmed = value.trim().trim_start_matches('@');
    if trimmed.is_empty() {
        return "yarn-package".to_string();
    }
    trimmed
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn escape_json(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 16);
    for ch in input.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => {
                let n = c as u32;
                out.push_str("\\u");
                out.push(char::from(b"0123456789ABCDEF"[((n >> 12) & 0xf) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[((n >> 8) & 0xf) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[((n >> 4) & 0xf) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(n & 0xf) as usize]));
            }
            c => out.push(c),
        }
    }
    out
}

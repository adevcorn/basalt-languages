#!/usr/bin/env python3
"""Generate registry index entries for language packs, presets, and providers.

Phase 2 helper. It does not modify the live registry by default:

  # write a standalone overlay with only the new entries
  python3 scripts/generate_registry.py --out /tmp/overlay.json

  # merge into a copy of the current index
  python3 scripts/generate_registry.py \
      --merge ../basalt-registry/index.json \
      --out /tmp/index.next.json

Artifact URLs point at the monorepo release for `--tag` (default v0.1.0).
SHA-256 values are included when the matching file exists in `dist/`.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import tomllib
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REPO = "https://github.com/adevcorn/basalt-languages"

# Existing registry name -> release asset.
PROVIDER_ASSETS = {
    "parser-csharp": "cs.wasm",
    "parser-kotlin": "kt.wasm",
    "parser-gleam": "gleam.wasm",
    "parser-typescript": "ts.wasm",
    "parser-yaml": "yaml.wasm",
    "parser-rust": "rs.wasm",
    "parser-swift": "swift.wasm",
    "parser-c": "c.wasm",
    "parser-markdown": "md.wasm",
    "parser-toml": "toml.wasm",
    "semantic-facts-csharp": "semantic_facts_csharp.wasm",
    "semantic-facts-kotlin": "semantic_facts_kotlin.wasm",
    "semantic-facts-gleam": "semantic_facts_gleam.wasm",
    "semantic-facts-typescript": "semantic_facts_typescript.wasm",
    "semantic-facts-rust": "semantic_facts_rust.wasm",
    "dotnet-project-model": "dotnet_project_model.wasm",
    "gradle-android-project-model": "gradle_android_project_model.wasm",
    "gleam-project-model": "gleam_project_model.wasm",
    "project-model-yarn-workspaces": "project_model_yarn_workspaces.wasm",
    "cargo-project-model": "cargo_project_model.wasm",
    "swift-project-model": "swift_project_model.wasm",
    "dotnet-api-index": "dotnet_api_index.wasm",
    "rust-review-actions": "rust_review_actions.wasm",
    "lsp-gleam": "lsp_gleam.wasm",
    "lsp-rust-analyzer": "lsp_rust_analyzer.wasm",
    "swift-lsp": "swift_lsp.wasm",
    "backstage-catalog": "backstage_catalog.wasm",
    "agent-backstage-yarn": "agent_backstage_yarn.wasm",
    "rust-ecs-inspector": "rust_ecs.wasm",
    "semantic-retrieval": "semantic_retrieval.wasm",
}

# Logical names used by presets -> historical registry entry name.
PROVIDER_ALIASES = {
    "dotnet-project-model": ["project-model-dotnet"],
    "gradle-android-project-model": ["project-model-gradle-android"],
    "gleam-project-model": ["project-model-gleam"],
    "cargo-project-model": ["project-model-cargo"],
    "swift-project-model": ["project-model-swift"],
    "dotnet-api-index": ["api-index-dotnet"],
    "rust-review-actions": ["review-actions-rust"],
    "swift-lsp": ["lsp-swift"],
}

# Logical preset member -> legacy registry entry name.
MEMBER_LEGACY = {
    "project-model-dotnet": "dotnet-project-model",
    "project-model-gradle-android": "gradle-android-project-model",
    "project-model-gleam": "gleam-project-model",
    "project-model-yarn-workspaces": "project-model-yarn-workspaces",
    "project-model-cargo": "cargo-project-model",
    "project-model-swift": "swift-project-model",
    "api-index-dotnet": "dotnet-api-index",
    "review-actions-rust": "rust-review-actions",
    "lsp-gleam": "lsp-gleam",
    "lsp-rust-analyzer": "lsp-rust-analyzer",
    "lsp-swift": "swift-lsp",
}

# Registry metadata corrections applied while merging.
CAPABILITY_OVERRIDES = {
    # swift-lsp exposes diagnostics/hover; the dedicated swift-project-model
    # owns PROJECT_MODEL.
    "swift-lsp": ["DIAGNOSTICS", "HOVER"],
}

PACK_LEGACY = {
    "language-csharp": ["parser-csharp", "semantic-facts-csharp"],
    "language-kotlin": ["parser-kotlin", "semantic-facts-kotlin"],
    "language-gleam": ["parser-gleam", "semantic-facts-gleam"],
    "language-typescript": ["parser-typescript", "semantic-facts-typescript"],
    "language-yaml": ["parser-yaml"],
    "language-rust": ["parser-rust", "semantic-facts-rust"],
    "language-swift": ["parser-swift"],
    "language-c": ["parser-c"],
    "language-markdown": ["parser-markdown"],
    "language-toml": ["parser-toml"],
}


def load(path: Path):
    return tomllib.loads(path.read_text())


def sha256(path: Path) -> str | None:
    if not path.exists():
        return None
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def package_versions() -> dict[str, str]:
    meta = json.loads(
        subprocess.check_output(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=ROOT, text=True
        )
    )
    return {pkg["name"]: pkg["version"] for pkg in meta["packages"]}


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--tag", default="v0.1.0")
    ap.add_argument("--base-url", default=None)
    ap.add_argument("--merge", type=Path, default=None, help="existing index.json to merge into")
    ap.add_argument("--out", type=Path, default=Path("/tmp/basalt-registry-overlay.json"))
    ap.add_argument("--dist", type=Path, default=ROOT / "dist", help="directory of release assets for checksums")
    args = ap.parse_args()

    base_url = args.base_url or f"{REPO}/releases/download/{args.tag}"
    artifacts = {a["artifact"]: a for a in load(ROOT / "release/artifacts.toml")["artifact"]}
    versions = package_versions()
    dist = args.dist

    plugins: list[dict] = []
    packs: list[dict] = []
    presets: list[dict] = []
    if args.merge:
        source = json.loads(args.merge.read_text())
        for entry in source.get("plugins", []):
            kind = entry.get("kind")
            if kind == "language-pack":
                packs.append(entry)
            elif kind == "preset":
                presets.append(entry)
            else:
                plugins.append(entry)
        packs.extend(source.get("packs", []))
        presets.extend(source.get("presets", []))
    by_name = {p["name"]: p for p in plugins + packs + presets}

    # 1. Update provider/parser/facts entries to point at the monorepo release.
    for name, asset in PROVIDER_ASSETS.items():
        entry = by_name.get(name)
        if entry is None:
            package = artifacts[asset]["package"]
            entry = {
                "name": name,
                "version": versions.get(package, "0.1.0"),
                "description": "",
                "author": "adevcorn",
                "homepage": REPO,
                "tags": [],
                "capabilities": [],
                "hook_flags": 0,
                "file_globs": [],
            }
            plugins.append(entry)
            by_name[name] = entry
        if name in CAPABILITY_OVERRIDES:
            entry["capabilities"] = CAPABILITY_OVERRIDES[name]
        entry["download_url"] = f"{base_url}/{asset}"
        entry["kind"] = "provider"
        entry["homepage"] = REPO
        if asset in artifacts:
            package = artifacts[asset]["package"]
            if package in versions:
                entry["version"] = versions[package]
            entry["target"] = artifacts[asset]["target"]
            digest = sha256(dist / asset)
            if digest:
                entry["sha256"] = digest
        if name in PROVIDER_ALIASES:
            aliases = set(entry.get("aliases", []))
            aliases.update(PROVIDER_ALIASES[name])
            entry["aliases"] = sorted(aliases)

    # 2. Language packs.
    for pack_path in sorted((ROOT / "packs").glob("*.toml")):
        pack = load(pack_path)["pack"]
        entries = []
        caps = set()
        for artifact in load(pack_path).get("artifact", []):
            asset = artifact["artifact"]
            meta = artifacts[asset]
            xmlns = {
                "kind": artifact.get("kind", meta["kind"]),
                "artifact": asset,
                "download_url": f"{base_url}/{asset}",
                "target": meta.get("target"),
            }
            if artifact.get("install_as"):
                xmlns["install_as"] = artifact["install_as"]
            if artifact.get("parser_filename"):
                xmlns["parser_filename"] = artifact["parser_filename"]
            digest = sha256(dist / asset)
            if digest:
                xmlns["sha256"] = digest
            entries.append(xmlns)
            if artifact.get("kind") == "parser":
                caps.add("SYNTAX_PARSER")
            if any(c.startswith("semantic-facts@") for c in pack.get("capabilities", [])):
                caps.add("SEMANTIC_FACTS")
        entry = {
            "name": pack["name"],
            "version": pack.get("version", "0.1.0"),
            "kind": "language-pack",
            "description": f"{pack.get('language', pack['name'])} language pack: parser and semantic facts",
            "author": "adevcorn",
            "homepage": REPO,
            "tags": ["language-pack", pack.get("language", "")],
            "capabilities": sorted(caps),
            "hook_flags": 0,
            "file_globs": pack.get("file_globs", []),
            "artifacts": entries,
        }
        entry["supersedes"] = PACK_LEGACY.get(pack["name"], [])
        if pack["name"] in by_name and by_name[pack["name"]] in packs:
            by_name[pack["name"]].update(entry)
        else:
            packs.append(entry)
            by_name[pack["name"]] = entry

    # 3. Presets.
    for preset_path in sorted((ROOT / "presets").glob("*.toml")):
        preset = load(preset_path)["preset"]
        members = []
        globs = list(preset.get("language_globs", []))
        for member in load(preset_path).get("member", []):
            members.append(
                {
                    "name": member["name"],
                    "version": member.get("version", ""),
                    "required": bool(member.get("required", False)),
                    "recommended": bool(member.get("recommended", False)),
                }
            )
        globs.extend(preset.get("markers", []))
        entry = {
            "name": preset["name"],
            "version": preset.get("version", "0.1.0"),
            "kind": "preset",
            "description": preset.get("description", ""),
            "author": "adevcorn",
            "homepage": REPO,
            "tags": ["preset", preset["name"]],
            "capabilities": [],
            "hook_flags": 0,
            "file_globs": sorted(set(globs)),
            "markers": preset.get("markers", []),
            "language_globs": preset.get("language_globs", []),
            "require_language_files": preset.get("require_language_files", True),
            "members": members,
        }
        supersedes = set()
        for member in members:
            legacy = MEMBER_LEGACY.get(member["name"])
            if legacy:
                supersedes.add(legacy)
            if member["name"].startswith("language-"):
                supersedes.add(member["name"])
        entry["supersedes"] = sorted(supersedes)
        if preset["name"] in by_name and by_name[preset["name"]] in presets:
            by_name[preset["name"]].update(entry)
        else:
            presets.append(entry)
            by_name[preset["name"]] = entry

    payload = {
        "version": "1",
        "updated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "plugins": plugins,
        "packs": packs,
        "presets": presets,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload, indent=2) + "\n")
    print(
        f"wrote {args.out}: {len(plugins)} plugins, {len(packs)} packs, {len(presets)} presets"
    )


if __name__ == "__main__":
    main()

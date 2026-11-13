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

    existing = []
    if args.merge:
        existing = json.loads(args.merge.read_text()).get("plugins", [])
    by_name = {p["name"]: p for p in existing}

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
            existing.append(entry)
            by_name[name] = entry
        entry["download_url"] = f"{base_url}/{asset}"
        entry["kind"] = "provider"
        entry["homepage"] = REPO
        if asset in artifacts:
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
        if pack["name"] in by_name:
            by_name[pack["name"]].update(entry)
        else:
            existing.append(entry)
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
        if preset["name"] in by_name:
            by_name[preset["name"]].update(entry)
        else:
            existing.append(entry)
            by_name[preset["name"]] = entry

    payload = {
        "version": "1",
        "updated_at": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "plugins": existing,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(payload, indent=2) + "\n")
    print(
        f"wrote {args.out}: {len(existing)} entries "
        f"({sum(1 for p in existing if p.get('kind') == 'language-pack')} packs, "
        f"{sum(1 for p in existing if p.get('kind') == 'preset')} presets)"
    )


if __name__ == "__main__":
    main()

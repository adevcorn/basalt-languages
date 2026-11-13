#!/usr/bin/env python3
"""Validate language-pack, preset, and artifact manifests before CI builds."""
import sys, tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
errors: list[str] = []

def load(path):
    return tomllib.loads(path.read_text())

artifacts = load(ROOT / "release/artifacts.toml").get("artifact", [])
by_artifact = {a["artifact"]: a for a in artifacts}
by_package = {a["package"]: a for a in artifacts}
pack_names = set()
plugin_names = {a["plugin_name"] for a in artifacts if a.get("plugin_name")}

PROVIDER_CAPS = ("project-model", "build-system", "test-runner", "review-action",
                 "api-index", "lsp", "diagnostics", "hover", "agent-launcher")

for path in sorted((ROOT / "packs").glob("*.toml")):
    p = load(path)
    pack = p.get("pack", {})
    name = pack.get("name", path.stem)
    pack_names.add(name)
    if not name.startswith("language-"):
        errors.append(f"{path}: pack name must start with 'language-': {name}")
    if not pack.get("file_globs"):
        errors.append(f"{path}: file_globs must not be empty")
    if not pack.get("capabilities"):
        errors.append(f"{path}: capabilities must not be empty")
    for cap in pack.get("capabilities", []):
        if not (cap.startswith("language@") or cap.startswith("semantic-facts@")):
            errors.append(f"{path}: language pack exposes non-language capability {cap!r}")
    arts = p.get("artifact", [])
    if not arts:
        errors.append(f"{path}: pack has no artifacts")
    if not any(a.get("kind") == "parser" for a in arts):
        errors.append(f"{path}: pack has no parser artifact")
    for a in arts:
        art = a.get("artifact")
        if art not in by_artifact:
            errors.append(f"{path}: unknown artifact {art!r}")
            continue
        meta = by_artifact[art]
        expected = "wasm32-wasip1" if a.get("kind") == "parser" else "wasm32-unknown-unknown"
        if meta.get("target") != expected:
            errors.append(f"{path}: {art} targets {meta.get('target')}, expected {expected}")
    if len({a.get("label", a.get("artifact")) for a in arts}) != len(arts):
        errors.append(f"{path}: duplicate artifacts")

for path in sorted((ROOT / "presets").glob("*.toml")):
    p = load(path)
    preset = p.get("preset", {})
    members = p.get("member", [])
    if not members:
        errors.append(f"{path}: preset has no members")
    for m in members:
        nm = m.get("name")
        if nm not in pack_names and nm not in plugin_names:
            errors.append(f"{path}: unknown member {nm!r}")
    if not preset.get("addon") and not any(
        m.get("required") and m.get("name") in pack_names for m in members
    ):
        errors.append(f"{path}: preset must require at least one language pack")
    if not preset.get("language_globs"):
        errors.append(f"{path}: language_globs must not be empty")
    if "require_language_files" in preset and not isinstance(preset["require_language_files"], bool):
        errors.append(f"{path}: require_language_files must be a boolean")

# every artifact is in exactly one pack or is an optional add-on/provider
pack_artifacts = {a["artifact"] for path in (ROOT / "packs").glob("*.toml")
                  for a in load(path).get("artifact", [])}
for a in artifacts:
    if a["kind"] == "parser" and a["artifact"] not in pack_artifacts:
        errors.append(f"release/artifacts.toml: parser {a['artifact']} is not in any language pack")

if errors:
    print("manifest validation failed:", file=sys.stderr)
    for e in errors:
        print("  - " + e, file=sys.stderr)
    sys.exit(1)
print(f"manifests ok: {len(pack_names)} packs, "
      f"{len(list((ROOT/'presets').glob('*.toml')))} presets, {len(artifacts)} artifacts")

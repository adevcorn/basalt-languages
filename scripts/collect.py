#!/usr/bin/env python3
"""Copy built wasm files from target/<triple>/release into dist/."""
import argparse, json, shutil, subprocess, sys, tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DIST = ROOT / "dist"
ap = argparse.ArgumentParser()
ap.add_argument("--package", help="collect only this package's artifact")
args = ap.parse_args()

DATA = tomllib.loads((ROOT / "release" / "artifacts.toml").read_text())
ARTIFACTS = DATA.get("artifact", [])
if args.package:
    ARTIFACTS = [a for a in ARTIFACTS if a["package"] == args.package]
    if not ARTIFACTS:
        sys.exit(f"unknown package: {args.package}")

meta = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--format-version", "1", "--no-deps"], cwd=ROOT, text=True
))
lib_names = {}
for pkg in meta["packages"]:
    for target in pkg["targets"]:
        if "cdylib" in target.get("kind", []):
            lib_names[pkg["name"]] = target["name"]

DIST.mkdir(exist_ok=True)
missing = []
for a in ARTIFACTS:
    pkg, out, triple = a["package"], a["artifact"], a["target"]
    lib = lib_names.get(pkg)
    if not lib:
        missing.append(f"{pkg}: no cdylib target")
        continue
    src = ROOT / "target" / triple / "release" / f"{lib}.wasm"
    if not src.exists():
        missing.append(f"{pkg}: {src.relative_to(ROOT)} not found")
        continue
    shutil.copy2(src, DIST / out)
    print(f"{pkg} [{triple}] -> dist/{out} ({src.stat().st_size} bytes)")

if missing:
    print("\nmissing artifacts:", file=sys.stderr)
    for m in missing:
        print("  " + m, file=sys.stderr)
    sys.exit(1)
print(f"collected {len(ARTIFACTS)} artifacts into {DIST}")

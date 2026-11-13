#!/usr/bin/env python3
"""Read release/artifacts.toml and emit CI matrix, lists, or per-package targets."""
import argparse, json, tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DATA = tomllib.loads((ROOT / "release" / "artifacts.toml").read_text())
ARTIFACTS = DATA.get("artifact", [])

def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--matrix", action="store_true", help="emit GitHub Actions matrix JSON")
    ap.add_argument("--names", action="store_true", help="emit artifact filenames only")
    ap.add_argument("--target-for-package", metavar="PKG", help="print the wasm target for a package")
    ap.add_argument("--packages", action="store_true", help="emit 'package target' lines")
    args = ap.parse_args()

    if args.target_for_package:
        for a in ARTIFACTS:
            if a["package"] == args.target_for_package:
                print(a["target"])
                return
        raise SystemExit(f"unknown package: {args.target_for_package}")
    if args.names:
        for a in ARTIFACTS:
            print(a["artifact"])
        return
    if args.packages:
        for a in ARTIFACTS:
            print(a["package"], a["target"])
        return
    payload = {
        "include": [
            {
                "package": a["package"],
                "artifact": a["artifact"],
                "kind": a["kind"],
                "target": a["target"],
            }
            for a in ARTIFACTS
        ]
    }
    print(json.dumps(payload, separators=(",", ":")))

if __name__ == "__main__":
    main()

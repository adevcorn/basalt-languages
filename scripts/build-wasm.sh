#!/usr/bin/env bash
# Build Basalt language-pack artifacts and collect them in dist/.
#
# Parsers target wasm32-wasip1 (WASI-enabled parser linker).
# Facts/providers target wasm32-unknown-unknown (capability-plugin linker).
#
# Usage:
#   ./scripts/build-wasm.sh                 # build every artifact
#   ./scripts/build-wasm.sh --package PKG   # build one package
#   WASI_SDK=/path/to/wasi-sdk ./scripts/build-wasm.sh
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PKG=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --package|-p)
      PKG="$2"; shift 2 ;;
    *)
      echo "usage: $0 [--package PKG]" >&2; exit 2 ;;
  esac
done

need_wasi=1
if [[ -n "$PKG" ]]; then
  target="$(python3 "$ROOT/scripts/artifacts.py" --target-for-package "$PKG")"
  [[ "$target" == "wasm32-wasip1" ]] || need_wasi=0
fi

setup_wasi() {
  if [[ -n "${WASI_SDK:-}" ]]; then
    CC="${WASI_SDK}/bin/clang"
    AR="${WASI_SDK}/bin/llvm-ar"
    SYSROOT="${WASI_SDK}/share/wasi-sysroot"
  elif [[ -d /opt/homebrew/opt/wasi-libc/share/wasi-sysroot ]]; then
    SYSROOT="/opt/homebrew/opt/wasi-libc/share/wasi-sysroot"
    if [[ -x /opt/homebrew/opt/llvm/bin/clang ]]; then
      CC="/opt/homebrew/opt/llvm/bin/clang"
      AR="/opt/homebrew/opt/llvm/bin/llvm-ar"
    else
      CC="$(command -v clang)"
      AR="$(command -v llvm-ar)"
    fi
  else
    CC="${CC:-$(command -v clang)}"
    AR="${AR:-$(command -v llvm-ar)}"
    SYSROOT="${WASI_SYSROOT:?set WASI_SDK or WASI_SYSROOT (wasi-libc sysroot)}"
  fi

  export CC_wasm32_wasip1="$CC"
  export AR_wasm32_wasip1="$AR"
  export CFLAGS_wasm32_wasip1="--sysroot=${SYSROOT} -include ${ROOT}/ci/wasi_dup_stub.h"
  echo "CC=$CC"
  echo "AR=$AR"
  echo "sysroot=$SYSROOT"
}

if [[ "$need_wasi" -eq 1 ]]; then
  setup_wasi
else
  echo "no WASI toolchain needed for this build"
fi

cd "$ROOT"

build_pkg() {
  local pkg="$1"
  local triple
  triple="$(python3 scripts/artifacts.py --target-for-package "$pkg")"
  echo "==> $pkg ($triple)"
  cargo build --release --target "$triple" -p "$pkg"
}

if [[ -n "$PKG" ]]; then
  build_pkg "$PKG"
  python3 scripts/collect.py --package "$PKG"
  exit 0
fi
# Deduplicate packages; each package appears once in the manifest.
python3 scripts/artifacts.py --packages | sort -u | while read -r pkg _target; do
  build_pkg "$pkg"
done

python3 scripts/collect.py

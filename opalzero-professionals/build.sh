#!/usr/bin/env bash
# Build all OpalZero professionals and install them into opalzero-core.
#
# Usage:
#   ./build.sh           — build all tools
#   ./build.sh calculator memory   — build specific tools by name
#
# Prerequisites:
#   rustup target add wasm32-wasip1
#
# Each tool is a standalone Rust crate (separate [workspace]) that compiles to
# a wasm32-wasip1 binary.  After building, both the .wasm binary and the
# manifest.json are copied to opalzero-core/professionals/ so the kernel can
# load them at startup.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CORE_PROFESSIONALS="$SCRIPT_DIR/../opalzero-core/professionals"
TARGET="wasm32-wasip1"

# Tools to build: either all subdirectories with a Cargo.toml, or the ones
# explicitly passed as arguments.
if [ $# -gt 0 ]; then
    TOOLS=("$@")
else
    TOOLS=()
    for dir in "$SCRIPT_DIR"/*/; do
        name=$(basename "$dir")
        if [ -f "$dir/Cargo.toml" ]; then
            TOOLS+=("$name")
        fi
    done
fi

ok=0
fail=0

for tool in "${TOOLS[@]}"; do
    src="$SCRIPT_DIR/$tool"
    if [ ! -d "$src" ]; then
        echo "  [skip] $tool — directory not found"
        continue
    fi

    echo "Building $tool..."

    # Build the WASM binary.
    if (cd "$src" && cargo build --target "$TARGET" --release --quiet 2>&1); then
        # Locate the compiled binary (name derived from package name, hyphens → underscores).
        pkg_name=$(grep '^name' "$src/Cargo.toml" | head -1 | sed 's/.*= *"//' | sed 's/".*//')
        bin_name="${pkg_name//-/_}"
        wasm_src="$src/target/$TARGET/release/${bin_name}.wasm"

        if [ ! -f "$wasm_src" ]; then
            echo "  [warn] $tool — .wasm not found at $wasm_src, skipping binary copy"
        else
            cp "$wasm_src" "$CORE_PROFESSIONALS/${tool}.wasm"
            echo "  copied ${tool}.wasm"
        fi
    else
        echo "  [warn] $tool — cargo build failed, skipping binary copy"
        ((fail++)) || true
        continue
    fi

    # Copy the manifest (always present alongside source).
    if [ -f "$src/manifest.json" ]; then
        cp "$src/manifest.json" "$CORE_PROFESSIONALS/manifests/${tool}.json"
        echo "  copied ${tool}.json"
    else
        echo "  [warn] $tool — manifest.json not found, skipping manifest copy"
    fi

    ((ok++)) || true
done

echo ""
echo "Done: $ok built, $fail failed."

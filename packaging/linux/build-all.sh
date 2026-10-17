#!/usr/bin/env bash
# ==============================================================================
# Wflow Master Linux Installer & Package Builder
# Builds:
#   1. Debian Package (.deb)
#   2. Portable Tarball (.tar.gz)
#   3. Universal Self-Extracting Installer (.run)
# ==============================================================================
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

cd "$ROOT_DIR"

echo "=========================================================="
echo "  🚀 Wflow Linux All-in-One Package Builder"
echo "=========================================================="

# 1. Compile release binary once
echo ">> Step 1/3: Compiling Release Binary with Cargo..."
cargo build --release

# 2. Build Debian package
echo ">> Step 2/3: Building Debian Package (.deb)..."
bash "$SCRIPT_DIR/build-deb.sh"

# 3. Build Portable Tarball and .run Installer
echo ">> Step 3/3: Building Portable Tarball and .run Installer..."
bash "$SCRIPT_DIR/build-run.sh"

echo ""
echo "=========================================================="
echo "  🎉 All Linux Packages Built Successfully in dist/:"
echo "=========================================================="
ls -lh dist/
echo "=========================================================="

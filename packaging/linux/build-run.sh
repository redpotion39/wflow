#!/usr/bin/env bash
# ==============================================================================
# Wflow Self-Extracting (.run) and Tarball Package Builder
# ==============================================================================
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

cd "$ROOT_DIR"

VERSION="0.1.0"
DIST_DIR="$ROOT_DIR/dist"
mkdir -p "$DIST_DIR"

echo "=========================================================="
echo "  Building Wflow Linux Release Packages (v$VERSION)"
echo "=========================================================="

# 1. Compile release binary if needed
if [ ! -f "target/release/wflow" ] || [ "$1" == "--rebuild" ]; then
    echo ">> Compiling release binary with cargo..."
    cargo build --release
fi

# 2. Prepare staging bundle
STAGING_DIR="target/wflow_bundle"
rm -rf "$STAGING_DIR"
mkdir -p "$STAGING_DIR"

echo ">> Preparing release files..."
cp target/release/wflow "$STAGING_DIR/wflow"
chmod 755 "$STAGING_DIR/wflow"
strip --strip-unneeded "$STAGING_DIR/wflow"

cp assets/icon_256.png "$STAGING_DIR/icon_256.png"
chmod 644 "$STAGING_DIR/icon_256.png"

cp packaging/linux/wflow.desktop "$STAGING_DIR/wflow.desktop"
chmod 644 "$STAGING_DIR/wflow.desktop"

cp install.sh "$STAGING_DIR/install.sh"
chmod 755 "$STAGING_DIR/install.sh"

cp uninstall.sh "$STAGING_DIR/uninstall.sh"
chmod 755 "$STAGING_DIR/uninstall.sh"

cat << 'EOF' > "$STAGING_DIR/README.txt"
==========================================================
  Wflow - High-performance AI Voice Dictation Studio
==========================================================
To install for current user (~/.local, no root required):
  ./install.sh --user

To install system-wide (/usr/local, requires sudo):
  sudo ./install.sh --system

To uninstall:
  ./uninstall.sh --user     (or sudo ./uninstall.sh --system)

To run directly without installing:
  ./wflow
==========================================================
EOF

# 3. Create .tar.gz bundle
TARBALL="$DIST_DIR/wflow-${VERSION}-linux-x86_64.tar.gz"
echo ">> Creating tarball: $TARBALL..."
tar -czf "$TARBALL" -C "$STAGING_DIR" .

# 4. Create self-extracting .run installer
RUN_INSTALLER="$DIST_DIR/wflow-${VERSION}-linux-x86_64.run"
echo ">> Creating self-extracting installer: $RUN_INSTALLER..."

HEADER_FILE="target/installer_header.sh"
cat << 'HEADER_EOF' > "$HEADER_FILE"
#!/usr/bin/env bash
# ==============================================================================
# Wflow Universal Self-Extracting Linux Installer
# ==============================================================================
set -e

TMP_DIR="$(mktemp -d /tmp/wflow-install-XXXXXX)"
cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT INT TERM

# Find payload binary start line
PAYLOAD_LINE=$(grep -a -n '^__PAYLOAD_BEGIN__$' "$0" | head -n 1 | cut -d: -f1)
if [ -z "$PAYLOAD_LINE" ]; then
    echo "Error: Corrupted installer file (payload marker missing)."
    exit 1
fi

tail -n +$((PAYLOAD_LINE + 1)) "$0" | tar -xz -C "$TMP_DIR"
"$TMP_DIR/install.sh" "$@"
exit $?

__PAYLOAD_BEGIN__
HEADER_EOF

cat "$HEADER_FILE" "$TARBALL" > "$RUN_INSTALLER"
chmod +x "$RUN_INSTALLER"
rm -f "$HEADER_FILE"

echo "=========================================================="
echo "  Linux Packages Built Successfully!"
echo "=========================================================="
echo "  • Tarball:     $TARBALL ($(du -h "$TARBALL" | cut -f1))"
echo "  • .run Script: $RUN_INSTALLER ($(du -h "$RUN_INSTALLER" | cut -f1))"
echo "=========================================================="

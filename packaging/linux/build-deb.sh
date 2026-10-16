#!/usr/bin/env bash
# Wflow Debian (.deb) Package Builder for Ubuntu/Debian
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"

cd "$ROOT_DIR"

echo "=========================================================="
echo "  Building Wflow Debian Package (.deb) for Ubuntu/Debian"
echo "=========================================================="

# 1. Build release binary if not built or clean build requested
if [ ! -f "target/release/wflow" ] || [ "$1" == "--rebuild" ]; then
    echo ">> Compiling release binary with cargo..."
    cargo build --release
fi

# 2. Setup Debian package tree
STAGING_DIR="target/debian_pkg"
echo ">> Setting up Debian packaging layout in $STAGING_DIR..."
rm -rf "$STAGING_DIR"

mkdir -p "$STAGING_DIR/DEBIAN"
mkdir -p "$STAGING_DIR/usr/bin"
mkdir -p "$STAGING_DIR/usr/share/applications"
mkdir -p "$STAGING_DIR/usr/share/icons/hicolor/256x256/apps"
mkdir -p "$STAGING_DIR/usr/share/pixmaps"
mkdir -p "$STAGING_DIR/usr/share/doc/wflow"

# 3. Copy application files
echo ">> Copying executable and desktop assets..."
cp target/release/wflow "$STAGING_DIR/usr/bin/wflow"
chmod 755 "$STAGING_DIR/usr/bin/wflow"

cp assets/icon_256.png "$STAGING_DIR/usr/share/icons/hicolor/256x256/apps/wflow.png"
chmod 644 "$STAGING_DIR/usr/share/icons/hicolor/256x256/apps/wflow.png"

cp assets/icon_256.png "$STAGING_DIR/usr/share/pixmaps/wflow.png"
chmod 644 "$STAGING_DIR/usr/share/pixmaps/wflow.png"

cp packaging/linux/wflow.desktop "$STAGING_DIR/usr/share/applications/wflow.desktop"
chmod 644 "$STAGING_DIR/usr/share/applications/wflow.desktop"

# 4. Copy DEBIAN control and maintainer scripts
echo ">> Setting up Debian control scripts..."
cp packaging/linux/DEBIAN/control "$STAGING_DIR/DEBIAN/control"
chmod 644 "$STAGING_DIR/DEBIAN/control"

cp packaging/linux/DEBIAN/postinst "$STAGING_DIR/DEBIAN/postinst"
chmod 755 "$STAGING_DIR/DEBIAN/postinst"

cp packaging/linux/DEBIAN/postrm "$STAGING_DIR/DEBIAN/postrm"
chmod 755 "$STAGING_DIR/DEBIAN/postrm"

# 5. Build .deb package using dpkg-deb
mkdir -p dist
OUTPUT_DEB="dist/wflow_0.1.0_amd64.deb"

echo ">> Packing Debian archive..."
dpkg-deb --build --root-owner-group "$STAGING_DIR" "$OUTPUT_DEB"

echo "=========================================================="
echo "  Package built successfully: $OUTPUT_DEB"
echo "  Size: $(du -h "$OUTPUT_DEB" | cut -f1)"
echo "=========================================================="
echo "Install on Ubuntu with:"
echo "  sudo dpkg -i $OUTPUT_DEB"
echo "  sudo apt-get install -f   # (if any missing dependencies)"
echo "=========================================================="

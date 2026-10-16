#!/usr/bin/env bash
# ==============================================================================
# Wflow Universal Linux / Ubuntu Installer
# High-performance native Rust AI voice dictation studio
# ==============================================================================
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if [ "$EUID" -ne 0 ]; then
  echo "Error: This installer requires root privileges."
  echo "Please run: sudo ./install.sh"
  exit 1
fi

echo "=========================================================="
echo "  Installing Wflow for Ubuntu / Linux"
echo "=========================================================="

# 1. Check or build binary
if [ ! -f "target/release/wflow" ]; then
    echo ">> target/release/wflow not found. Checking Rust toolchain..."
    if which cargo >/dev/null 2>&1; then
        echo ">> Compiling Wflow with cargo..."
        cargo build --release
    else
        echo "Error: cargo is not installed and prebuilt target/release/wflow was not found."
        echo "Please install Rust (curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh) or download the prebuilt .deb package."
        exit 1
    fi
fi

# 2. Install binary to /usr/local/bin
echo ">> Installing binary to /usr/local/bin/wflow..."
install -d /usr/local/bin
install -m 755 target/release/wflow /usr/local/bin/wflow

# 3. Install icons
echo ">> Installing application icons..."
install -d /usr/share/icons/hicolor/256x256/apps
install -m 644 assets/icon_256.png /usr/share/icons/hicolor/256x256/apps/wflow.png

install -d /usr/share/pixmaps
install -m 644 assets/icon_256.png /usr/share/pixmaps/wflow.png

# 4. Install Desktop Launcher
echo ">> Installing desktop launcher..."
install -d /usr/share/applications
install -m 644 packaging/linux/wflow.desktop /usr/share/applications/wflow.desktop

# 5. Update system caches
echo ">> Updating desktop database and icon caches..."
if which update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications || true
fi

if which gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
fi

echo "=========================================================="
echo "  🎉 Wflow has been successfully installed!"
echo "=========================================================="
echo "  • Run directly from terminal: wflow"
echo "  • Launch from Application Menu: Search for 'Wflow'"
echo "  • Background Tray Autostart: wflow --startup"
echo "=========================================================="

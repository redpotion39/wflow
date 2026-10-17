#!/usr/bin/env bash
# ==============================================================================
# Wflow Universal Linux / Ubuntu Installer
# High-performance native Rust AI voice dictation studio
# ==============================================================================
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

MODE=""
for arg in "$@"; do
    case "$arg" in
        --user)
            MODE="user"
            ;;
        --system)
            MODE="system"
            ;;
        --help|-h)
            echo "Usage: ./install.sh [OPTIONS]"
            echo "Options:"
            echo "  --user       Install for current user only (~/.local/bin, no root/sudo needed)"
            echo "  --system     Install system-wide (/usr/local/bin, requires root/sudo)"
            echo "  --help, -h   Show this help message"
            exit 0
            ;;
    esac
done

if [ -z "$MODE" ]; then
    if [ "$EUID" -eq 0 ]; then
        MODE="system"
    else
        if [ -t 0 ]; then
            echo "=========================================================="
            echo "  Wflow Linux Installer"
            echo "=========================================================="
            echo "Running without root privileges."
            echo "  1) Install for current user only (~/.local) [No root needed]"
            echo "  2) Install system-wide (/usr/local) [Requires sudo]"
            echo ""
            read -r -p "Choose installation type [1/2] (Default: 1): " choice
            if [ "$choice" = "2" ]; then
                echo "Elevating privileges with sudo..."
                exec sudo "$0" --system "$@"
            else
                MODE="user"
            fi
        else
            MODE="user"
        fi
    fi
fi

if [ "$MODE" = "system" ] && [ "$EUID" -ne 0 ]; then
    echo "Error: System-wide installation requires root privileges."
    echo "Please run: sudo ./install.sh --system"
    exit 1
fi

echo "=========================================================="
echo "  Installing Wflow for Linux (Mode: $MODE)"
echo "=========================================================="

# 1. Locate binary or build
SRC_BIN=""
if [ -f "./wflow" ]; then
    SRC_BIN="./wflow"
elif [ -f "./bin/wflow" ]; then
    SRC_BIN="./bin/wflow"
elif [ -f "target/release/wflow" ]; then
    SRC_BIN="target/release/wflow"
else
    echo ">> Prebuilt wflow binary not found. Checking Rust toolchain..."
    if which cargo >/dev/null 2>&1; then
        echo ">> Compiling Wflow with cargo..."
        cargo build --release
        SRC_BIN="target/release/wflow"
    else
        echo "Error: cargo is not installed and prebuilt wflow binary was not found."
        echo "Please install Rust (curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh) or download the prebuilt release package."
        exit 1
    fi
fi

# 2. Locate icon and desktop entry
SRC_ICON=""
if [ -f "assets/icon_256.png" ]; then
    SRC_ICON="assets/icon_256.png"
elif [ -f "./icon_256.png" ]; then
    SRC_ICON="./icon_256.png"
elif [ -f "icons/icon_256.png" ]; then
    SRC_ICON="icons/icon_256.png"
fi

SRC_DESKTOP=""
if [ -f "packaging/linux/wflow.desktop" ]; then
    SRC_DESKTOP="packaging/linux/wflow.desktop"
elif [ -f "./wflow.desktop" ]; then
    SRC_DESKTOP="./wflow.desktop"
fi

# 3. Setup target directories
if [ "$MODE" = "system" ]; then
    BIN_DIR="/usr/local/bin"
    DESKTOP_DIR="/usr/share/applications"
    ICON_DIR="/usr/share/icons/hicolor/256x256/apps"
    PIXMAPS_DIR="/usr/share/pixmaps"
else
    BIN_DIR="$HOME/.local/bin"
    DESKTOP_DIR="$HOME/.local/share/applications"
    ICON_DIR="$HOME/.local/share/icons/hicolor/256x256/apps"
    PIXMAPS_DIR=""
fi

echo ">> Installing binary to $BIN_DIR/wflow..."
install -d "$BIN_DIR"
install -m 755 "$SRC_BIN" "$BIN_DIR/wflow"
strip --strip-unneeded "$BIN_DIR/wflow" 2>/dev/null || true

if [ -n "$SRC_ICON" ]; then
    echo ">> Installing application icons..."
    install -d "$ICON_DIR"
    install -m 644 "$SRC_ICON" "$ICON_DIR/wflow.png"
    if [ -n "$PIXMAPS_DIR" ]; then
        install -d "$PIXMAPS_DIR"
        install -m 644 "$SRC_ICON" "$PIXMAPS_DIR/wflow.png"
    fi
fi

if [ -n "$SRC_DESKTOP" ]; then
    echo ">> Installing desktop launcher..."
    install -d "$DESKTOP_DIR"
    if [ "$MODE" = "user" ]; then
        # Ensure desktop entry points directly to user's local bin
        sed "s|^Exec=wflow|Exec=$BIN_DIR/wflow|" "$SRC_DESKTOP" > "$DESKTOP_DIR/wflow.desktop"
        chmod 644 "$DESKTOP_DIR/wflow.desktop"
    else
        install -m 644 "$SRC_DESKTOP" "$DESKTOP_DIR/wflow.desktop"
    fi
fi

# 4. Update desktop & icon caches
echo ">> Updating desktop database and icon caches..."
if which update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q "$DESKTOP_DIR" 2>/dev/null || true
fi

if which gtk-update-icon-cache >/dev/null 2>&1; then
    if [ "$MODE" = "system" ]; then
        gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor 2>/dev/null || true
    else
        gtk-update-icon-cache -q -t -f "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
    fi
fi

echo "=========================================================="
echo "  🎉 Wflow has been successfully installed! ($MODE mode)"
echo "=========================================================="
echo "  • Binary: $BIN_DIR/wflow"
echo "  • Launcher: $DESKTOP_DIR/wflow.desktop"
if [ "$MODE" = "user" ]; then
    case ":$PATH:" in
        *":$BIN_DIR:"*) ;;
        *)
            echo "  ⚠️  Notice: $BIN_DIR is not currently in your PATH."
            echo "     Add it by running:"
            echo "       export PATH=\"$BIN_DIR:\$PATH\""
            echo "     or append it to your ~/.bashrc or ~/.zshrc."
            ;;
    esac
fi
echo "  • Run directly from terminal: wflow"
echo "  • Launch from Application Menu: Search for 'Wflow'"
echo "  • Background Tray Autostart: wflow --startup"
echo "=========================================================="

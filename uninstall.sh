#!/usr/bin/env bash
# ==============================================================================
# Wflow Linux / Ubuntu Uninstaller
# ==============================================================================
set -e

MODE=""
for arg in "$@"; do
    case "$arg" in
        --user)
            MODE="user"
            ;;
        --system)
            MODE="system"
            ;;
        --all)
            MODE="all"
            ;;
        --help|-h)
            echo "Usage: ./uninstall.sh [OPTIONS]"
            echo "Options:"
            echo "  --user       Uninstall from current user directory (~/.local)"
            echo "  --system     Uninstall system-wide (/usr/local, requires root/sudo)"
            echo "  --all        Uninstall from both user and system locations"
            echo "  --help, -h   Show this help message"
            exit 0
            ;;
    esac
done

if [ -z "$MODE" ]; then
    if [ "$EUID" -eq 0 ]; then
        MODE="system"
    else
        MODE="user"
    fi
fi

echo "=========================================================="
echo "  Uninstalling Wflow (Mode: $MODE)"
echo "=========================================================="

if [ "$MODE" = "system" ] || [ "$MODE" = "all" ]; then
    if [ "$EUID" -ne 0 ] && [ "$MODE" = "system" ]; then
        echo "Error: System-wide uninstallation requires root privileges."
        echo "Please run: sudo ./uninstall.sh --system"
        exit 1
    fi
    echo ">> Removing system-wide files..."
    rm -f /usr/local/bin/wflow /usr/bin/wflow
    rm -f /usr/share/applications/wflow.desktop
    rm -f /usr/share/icons/hicolor/256x256/apps/wflow.png
    rm -f /usr/share/pixmaps/wflow.png
    if which update-desktop-database >/dev/null 2>&1; then
        update-desktop-database -q /usr/share/applications 2>/dev/null || true
    fi
    if which gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor 2>/dev/null || true
    fi
fi

if [ "$MODE" = "user" ] || [ "$MODE" = "all" ]; then
    echo ">> Removing user-local files..."
    rm -f "$HOME/.local/bin/wflow"
    rm -f "$HOME/.local/share/applications/wflow.desktop"
    rm -f "$HOME/.local/share/icons/hicolor/256x256/apps/wflow.png"
    if which update-desktop-database >/dev/null 2>&1; then
        update-desktop-database -q "$HOME/.local/share/applications" 2>/dev/null || true
    fi
    if which gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -q -t -f "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
    fi
fi

echo "=========================================================="
echo "  Wflow has been successfully uninstalled."
echo "=========================================================="

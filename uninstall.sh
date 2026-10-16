#!/usr/bin/env bash
# ==============================================================================
# Wflow Linux / Ubuntu Uninstaller
# ==============================================================================
set -e

if [ "$EUID" -ne 0 ]; then
  echo "Error: This uninstaller requires root privileges."
  echo "Please run: sudo ./uninstall.sh"
  exit 1
fi

echo "Uninstalling Wflow..."

rm -f /usr/local/bin/wflow /usr/bin/wflow
rm -f /usr/share/applications/wflow.desktop
rm -f /usr/share/icons/hicolor/256x256/apps/wflow.png
rm -f /usr/share/pixmaps/wflow.png

if which update-desktop-database >/dev/null 2>&1; then
    update-desktop-database -q /usr/share/applications || true
fi

if which gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor || true
fi

echo "Wflow has been successfully uninstalled from your system."

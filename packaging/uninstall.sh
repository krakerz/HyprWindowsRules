#!/usr/bin/env bash
# Removes what install.sh installed. Your rules (~/.config/hypr-rules/) and
# the hook lines in your Hyprland config are left alone — Hyprland keeps
# applying the rules without the app.
set -euo pipefail

bin_dir="$HOME/.local/bin"
apps_dir="$HOME/.local/share/applications"
icon_dir="$HOME/.local/share/icons/hicolor/scalable/apps"

removed=0
for f in "$bin_dir/hypr-rules" "$apps_dir/hypr-rules.desktop" "$icon_dir/hypr-rules.svg"; do
    if [ -e "$f" ]; then
        rm -f "$f"
        echo "Removed $f"
        removed=1
    fi
done

command -v update-desktop-database >/dev/null 2>&1 &&
    update-desktop-database "$apps_dir" 2>/dev/null || true

if [ "$removed" -eq 0 ]; then
    echo "Nothing installed by install.sh was found."
else
    echo "Done. Your rules at ~/.config/hypr-rules/ were left in place."
fi

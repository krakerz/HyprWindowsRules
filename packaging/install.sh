#!/usr/bin/env bash
# Installs hypr-rules (binary + desktop entry + icon) for the current user.
# Run from inside the extracted release archive, next to this script:
#   ./install.sh
# or from a clone of the repo (builds the binary with cargo first):
#   packaging/install.sh
set -euo pipefail

dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
bin_dir="$HOME/.local/bin"
apps_dir="$HOME/.local/share/applications"
icon_dir="$HOME/.local/share/icons/hicolor/scalable/apps"

bin="$dir/hypr-rules"
if [ ! -f "$bin" ]; then
    repo="$(dirname "$dir")"
    if [ -f "$repo/Cargo.toml" ] && command -v cargo >/dev/null 2>&1; then
        echo "Building hypr-rules (cargo build --release)…"
        (cd "$repo" && cargo build --release --locked)
        bin="$repo/target/release/hypr-rules"
    else
        echo "error: hypr-rules binary not found next to this script ($dir)," >&2
        echo "       and no Rust toolchain/repo to build it from." >&2
        exit 1
    fi
fi

mkdir -p "$bin_dir" "$apps_dir" "$icon_dir"

# Copy then rename: overwriting the binary in place fails ("text file busy")
# while the app is running, which is the case when it updates itself.
install -m 755 "$bin" "$bin_dir/.hypr-rules.new"
mv -f "$bin_dir/.hypr-rules.new" "$bin_dir/hypr-rules"
install -m 644 "$dir/hypr-rules.svg" "$icon_dir/hypr-rules.svg"

# Launchers don't reliably inherit ~/.local/bin on $PATH, so point the
# installed entry at the absolute binary path.
sed -e "s|^Exec=.*|Exec=$bin_dir/hypr-rules|" "$dir/hypr-rules.desktop" > "$apps_dir/hypr-rules.desktop"
chmod 644 "$apps_dir/hypr-rules.desktop"

command -v update-desktop-database >/dev/null 2>&1 &&
    update-desktop-database "$apps_dir" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null 2>&1 &&
    gtk-update-icon-cache -q "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

echo "Installed to $bin_dir/hypr-rules"

case ":$PATH:" in
*":$bin_dir:"*) ;;
*)
    echo "Note: $bin_dir isn't on your \$PATH — add it in your shell's rc file"
    echo "  (e.g. export PATH=\"\$HOME/.local/bin:\$PATH\") to run 'hypr-rules' directly."
    ;;
esac

echo "Done. Run with: hypr-rules (or 'Hyprland Windows Rules' in your app launcher)"
echo "Updates: the app offers new releases at start-up (Settings → Check now)."

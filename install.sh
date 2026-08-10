#!/usr/bin/env bash
# Install RawToDng into the current user's desktop environment.
# No root needed: everything lands under ~/.local.
set -euo pipefail

APP_ID="dk.lundmoller.RawToDng"
SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"
APP_DIR="$DATA_DIR/applications"
ICON_DIR="$DATA_DIR/icons/hicolor/scalable/apps"

if [[ "${1:-}" == "--uninstall" ]]; then
    rm -fv "$BIN_DIR/rawtodng" \
           "$ICON_DIR/$APP_ID.svg" \
           "$APP_DIR/$APP_ID.desktop"
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APP_DIR" || true
    command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -f -t "$DATA_DIR/icons/hicolor" 2>/dev/null || true
    echo "Removed. Your converted DNGs and originals were not touched."
    exit 0
fi

echo "Building release binary…"
cargo build --release --manifest-path "$SRC_DIR/Cargo.toml"

mkdir -p "$BIN_DIR" "$APP_DIR" "$ICON_DIR"

echo "Installing binary to $BIN_DIR/rawtodng"
install -m 755 "$SRC_DIR/target/release/rawtodng" "$BIN_DIR/rawtodng"

echo "Installing icon to $ICON_DIR/$APP_ID.svg"
install -m 644 "$SRC_DIR/data/$APP_ID.svg" "$ICON_DIR/$APP_ID.svg"

# The Exec path is written absolute, because ~/.local/bin is not reliably on
# PATH for apps launched by the GNOME shell.
echo "Installing desktop entry to $APP_DIR/$APP_ID.desktop"
sed "s|@BINARY@|$BIN_DIR/rawtodng|g" \
    "$SRC_DIR/data/$APP_ID.desktop.in" > "$APP_DIR/$APP_ID.desktop"
chmod 644 "$APP_DIR/$APP_ID.desktop"

if command -v desktop-file-validate >/dev/null 2>&1; then
    desktop-file-validate "$APP_DIR/$APP_ID.desktop" && echo "Desktop entry valid."
fi

# Registering MIME types can make this app the default handler and displace a
# raw editor, so double-clicking a NEF would open a converter instead of
# Darktable. Record the existing defaults and put back anything we displaced;
# the converter stays available under "Open With".
MIMES=$(sed -n 's/^MimeType=//p' "$SRC_DIR/data/$APP_ID.desktop.in" | tr ';' ' ')
declare -A PREVIOUS_DEFAULT
for mime in $MIMES; do
    current=$(xdg-mime query default "$mime" 2>/dev/null || true)
    if [ -n "$current" ] && [ "$current" != "$APP_ID.desktop" ]; then
        PREVIOUS_DEFAULT["$mime"]="$current"
    fi
done

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APP_DIR" || true

restored=0
for mime in "${!PREVIOUS_DEFAULT[@]}"; do
    now=$(xdg-mime query default "$mime" 2>/dev/null || true)
    if [ "$now" != "${PREVIOUS_DEFAULT[$mime]}" ]; then
        xdg-mime default "${PREVIOUS_DEFAULT[$mime]}" "$mime"
        restored=$((restored+1))
    fi
done
[ "$restored" -gt 0 ] && echo "Kept your existing default apps for $restored raw formats."
command -v gtk-update-icon-cache  >/dev/null 2>&1 && gtk-update-icon-cache -f -t "$DATA_DIR/icons/hicolor" 2>/dev/null || true

echo
echo "Installed. 'RAW to DNG' should now appear in your app grid."
echo "To remove:  $SRC_DIR/install.sh --uninstall"

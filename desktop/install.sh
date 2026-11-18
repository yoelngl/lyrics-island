#!/usr/bin/env sh
set -eu

case "$0" in
    -*) script_path="./$0" ;;
    *) script_path="$0" ;;
esac
SCRIPT_DIR=$(CDPATH= cd "$(dirname "$script_path")" && pwd)

BIN_DIR="${PREFIX:-$HOME/.local}/bin"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}"

BIN_DEST="$BIN_DIR/lyrics-island"
DESKTOP_DEST="$DATA_DIR/applications/lyrics-island.desktop"
ICON_DEST="$DATA_DIR/icons/hicolor/scalable/apps/lyrics-island.svg"

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$BIN_DEST" "$DESKTOP_DEST" "$ICON_DEST"
    exit 0
fi

if [ "$#" -gt 0 ]; then
    printf 'Usage: %s [--uninstall]\n' "$0" >&2
    exit 1
fi

if [ -f "$SCRIPT_DIR/target/release/lyrics-island" ]; then
    BIN_SRC="$SCRIPT_DIR/target/release/lyrics-island"
elif [ -f "$SCRIPT_DIR/lyrics-island" ]; then
    BIN_SRC="$SCRIPT_DIR/lyrics-island"
else
    (cd "$SCRIPT_DIR" && cargo build --release)
    BIN_SRC="$SCRIPT_DIR/target/release/lyrics-island"
fi

mkdir -p \
    "$BIN_DIR" \
    "$DATA_DIR/applications" \
    "$DATA_DIR/icons/hicolor/scalable/apps"

install -m 0755 "$BIN_SRC" "$BIN_DEST"
install -m 0644 "$SCRIPT_DIR/packaging/lyrics-island.svg" "$ICON_DEST"

case "$BIN_DEST" in
    *[[:space:]]*)
        ESCAPED_BIN=$(printf '%s' "$BIN_DEST" | sed 's/\\/\\\\/g; s/"/\\"/g')
        EXEC_LINE="Exec=\"$ESCAPED_BIN\""
        ;;
    *)
        EXEC_LINE="Exec=$BIN_DEST"
        ;;
esac

while IFS= read -r line || [ -n "$line" ]; do
    case "$line" in
        Exec=*) printf '%s\n' "$EXEC_LINE" ;;
        *) printf '%s\n' "$line" ;;
    esac
done < "$SCRIPT_DIR/packaging/lyrics-island.desktop" > "$DESKTOP_DEST"
chmod 0644 "$DESKTOP_DEST"

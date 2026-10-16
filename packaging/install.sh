#!/usr/bin/env bash
# Installe MusicApp pour ton compte utilisateur : binaire, icône et entrée de menu.
# Rien n'est écrit en dehors de ton dossier personnel, aucun droit administrateur n'est demandé.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_SRC="$ROOT/src-tauri/target/release/musicapp"

if [ ! -x "$BIN_SRC" ]; then
  echo "Le programme n'est pas encore compilé."
  echo "Lance d'abord :  cd $ROOT && ./packaging/build.sh"
  exit 1
fi

BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons/hicolor/512x512/apps"

mkdir -p "$BIN_DIR" "$APP_DIR" "$ICON_DIR"

install -m 755 "$BIN_SRC" "$BIN_DIR/musicapp"
install -m 644 "$ROOT/src-tauri/icons/icon.png" "$ICON_DIR/musicapp.png"

sed "s|@BIN@|$BIN_DIR/musicapp|" "$ROOT/packaging/musicapp.desktop.in" > "$APP_DIR/musicapp.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" >/dev/null 2>&1 || true
fi

echo "MusicApp est installé."
echo "Tu la trouveras dans ton menu d'applications, ou en tapant : musicapp"

#!/usr/bin/env bash
# Installe Écho pour ton compte utilisateur : binaire, icône et entrée de menu.
# Rien n'est écrit en dehors de ton dossier personnel, aucun droit administrateur n'est demandé.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_SRC="$ROOT/src-tauri/target/release/echo-player"

if [ ! -x "$BIN_SRC" ]; then
  echo "Le programme n'est pas encore compilé."
  echo "Lance d'abord :  cd $ROOT && ./packaging/build.sh"
  exit 1
fi

BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons/hicolor/512x512/apps"

mkdir -p "$BIN_DIR" "$APP_DIR" "$ICON_DIR"

install -m 755 "$BIN_SRC" "$BIN_DIR/echo-player"
install -m 644 "$ROOT/src-tauri/icons/icon.png" "$ICON_DIR/echo.png"

sed "s|@BIN@|$BIN_DIR/echo-player|" "$ROOT/packaging/echo.desktop.in" > "$APP_DIR/echo.desktop"

# Ancien nom de commande : « echo » se confondait avec la commande système echo.
# On ne retire que le binaire d'Écho (il contient son identifiant), jamais autre chose.
if [ -f "$BIN_DIR/echo" ] && grep -aq "com.echo.player" "$BIN_DIR/echo" 2>/dev/null; then
  rm -f "$BIN_DIR/echo"
  echo "L'ancienne commande « echo » a été retirée : elle masquait la commande système."
fi

# L'ancienne version s'appelait MusicApp : on enlève ses traces.
removed=0
for old in "$BIN_DIR/musicapp" "$APP_DIR/musicapp.desktop" "$ICON_DIR/musicapp.png"; do
  if [ -e "$old" ]; then
    rm -f "$old"
    removed=1
  fi
done
if [ "$removed" = "1" ]; then
  echo "L'ancienne version (MusicApp) a été désinstallée."
fi

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" >/dev/null 2>&1 || true
fi

echo "Écho est installé."
echo "Tu la trouveras dans ton menu d'applications, ou en tapant : echo-player"

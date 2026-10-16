#!/usr/bin/env bash
# Compile Écho. Aucun droit administrateur nécessaire.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Rust installé par mise (sinon, on prend celui qui est dans le PATH).
if [ -d "$HOME/.local/share/mise/shims" ]; then
  export PATH="$HOME/.local/share/mise/shims:$PATH"
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust n'est pas installé. Installe-le puis relance."
  exit 1
fi

cd "$ROOT/src-tauri"
cargo build --release --features custom-protocol
echo
echo "C'est compilé : $ROOT/src-tauri/target/release/echo-player"

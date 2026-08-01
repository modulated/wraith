#!/bin/bash
#
# Install Wraith Language Support for VS Code: syntax highlighting AND the
# language server (diagnostics, completion, hover).
#
# It builds the server, compiles the extension client, and links the extension
# into your VS Code extensions directory. Re-run it after pulling changes.

set -e

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/.." && pwd)"

echo "==> Building the language server (release)…"
( cd "$REPO_ROOT" && cargo build --release -p wraith-lsp )
SERVER_BIN="$REPO_ROOT/target/release/wraith-lsp"
echo "    server: $SERVER_BIN"

echo "==> Installing client dependencies and compiling…"
( cd "$HERE" && npm install --no-audit --no-fund && npm run compile )

# VS Code extensions directory.
case "$OSTYPE" in
  darwin*|linux-gnu*) VSCODE_EXT_DIR="$HOME/.vscode/extensions" ;;
  msys*|win32*)       VSCODE_EXT_DIR="$USERPROFILE/.vscode/extensions" ;;
  *) echo "Unsupported OS: $OSTYPE"; exit 1 ;;
esac

EXT_PATH="$VSCODE_EXT_DIR/wraith-language-0.2.0"
echo "==> Linking the extension into $EXT_PATH"
mkdir -p "$VSCODE_EXT_DIR"
rm -rf "$EXT_PATH"
ln -s "$HERE" "$EXT_PATH"

echo ""
echo "✅ Done."
echo ""
echo "Point the extension at the server. Either add it to your PATH:"
echo "    export PATH=\"$REPO_ROOT/target/release:\$PATH\""
echo "or set this in VS Code settings.json:"
echo "    \"wraith.server.path\": \"$SERVER_BIN\""
echo ""
echo "Then reload VS Code (Developer: Reload Window) and open a .wr file."

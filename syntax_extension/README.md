# Wraith Language Support for VS Code

Two things in one extension:

- **Syntax highlighting** — a TextMate grammar (`wraith.tmLanguage.json`). No
  build, no server; works on its own.
- **Language features** — diagnostics, completion, and hover, from the
  `wraith-lsp` language server. The client here just launches it and forwards
  LSP traffic; what the server does and how it works is documented in
  [`../crates/wraith-lsp/README.md`](../crates/wraith-lsp/README.md).

## Install

```sh
./install-vscode.sh
```

That builds the server (`cargo build --release -p wraith-lsp`), compiles the
client (`npm install && npm run compile`), and links the extension into
`~/.vscode/extensions`. Then point the extension at the server — either add
`target/release` to your `PATH`, or set in VS Code `settings.json`:

```json
{ "wraith.server.path": "/absolute/path/to/target/release/wraith-lsp" }
```

Reload VS Code and open a `.wr` file.

### Manual build

```sh
cargo build --release -p wraith-lsp   # from the repo root
cd syntax_extension && npm install && npm run compile
```

## Settings

- `wraith.server.path` — path to the `wraith-lsp` binary (default: looked up on
  `PATH`).
- `wraith.server.enable` — set to `false` to keep highlighting only. Highlighting
  is the TextMate grammar and works with or without the server.

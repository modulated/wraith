# Wraith Language Support for VS Code

Two things in one extension:

- **Syntax highlighting** — a TextMate grammar (`wraith.tmLanguage.json`). No
  build, no server; works on its own.
- **Language server** — diagnostics, completion, and hover, provided by the
  `wraith-lsp` binary (in `crates/wraith-lsp`), which reuses the Wraith compiler.

## What the server does today

| Feature | Notes |
| --- | --- |
| **Diagnostics** | Errors (lex / parse / type) and warnings (unused, non-exhaustive match, …), live as you type. Fail-fast, so one error shows at a time. |
| **Completion** | After `.`: the fields of the receiver's struct. Otherwise: keywords, primitive types, and the file's top-level declarations. |
| **Hover** | The type of the expression under the cursor. |

Completion of **locals and parameters in general position** is not in yet — it
needs a scope-at-position query the batch compiler does not expose. Field
completion, which is the part that needs types, works.

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

## Manual build

```sh
cargo build --release -p wraith-lsp   # from the repo root
cd syntax_extension && npm install && npm run compile
```

## Settings

- `wraith.server.path` — path to the `wraith-lsp` binary (default: looked up on
  `PATH`).
- `wraith.server.enable` — set to `false` to keep highlighting only.

## How it fits together

```
VS Code ──stdio(LSP)──▶ wraith-lsp ──library call──▶ wraith compiler
  (client extension)      (server)                    (lex/parse/analyze)
```

The client (`src/extension.ts`) is thin: it launches the server and forwards
LSP traffic. All language intelligence lives in the server, which calls the
compiler's `lex` → `parse` → `analyze` on each edit and shapes the result into
LSP diagnostics/completions/hovers. On a buffer that does not parse, completion
and hover fall back to the last analysis that succeeded, so they keep working
mid-edit.

# wraith-lsp

The Wraith language server: diagnostics, completion, and hover, spoken over the
Language Server Protocol. It reuses the Wraith compiler as a library, so what it
reports is exactly what the compiler knows.

The VS Code client that launches this server lives in
[`../../syntax_extension`](../../syntax_extension); see its README to install
the extension. This document is about the server itself.

## What it does today

| Feature | Notes |
| --- | --- |
| **Diagnostics** | Errors (lex / parse / type) and warnings (unused, non-exhaustive match, …), live as you type. Fail-fast, so one error shows at a time. |
| **Completion** | After `.`: the fields of the receiver's struct. Otherwise: keywords, primitive types, and the file's top-level declarations. |
| **Hover** | The type of the narrowest expression under the cursor. |

Completion of **locals and parameters in general position** is not in yet — it
needs a scope-at-position query the batch compiler does not expose. Field
completion, which is the part that needs types, works. There is also no
go-to-definition or rename yet. Those are the natural next slices.

## How it fits together

```
VS Code ──stdio (LSP)──▶ wraith-lsp ──library call──▶ wraith compiler
  (client extension)       (server)                    (lex / parse / analyze)
```

The client is thin: it launches this server and forwards LSP traffic. All
language intelligence lives here. On each edit the server runs the compiler's
`lex → parse → analyze` over the whole document and shapes the result into LSP
diagnostics, completions, and hovers.

## Design

- **Synchronous, no incrementality.** Built on `lsp-server` (not `tower-lsp`),
  with no async runtime. Each edit re-lexes, re-parses, and re-analyzes the
  whole file. That is affordable because the front-end runs in single-digit
  milliseconds on the file sizes this language sees (a few hundred to a couple
  thousand lines), and it keeps the server simple. The client is expected to
  debounce.
- **Last-good analysis.** Completion and hover read from the last analysis that
  *succeeded*, so they keep working while the current buffer is mid-edit and does
  not parse — which is most of the time you actually want them.
- **Proper position mapping.** LSP counts UTF-16 code units; the compiler speaks
  byte offsets. `line_index.rs` converts between them correctly rather than
  assuming ASCII.
- **Isolation.** The LSP dependencies (`lsp-server`, `lsp-types`, …) live only in
  this crate. The compiler's own build never sees them. Two small tooling
  accessors were added to the compiler — `SemaError::span()` and
  `Warning::span()`/`parts()` — so diagnostics land on a precise range instead of
  being parsed out of a formatted string.

### Modules

| File | Responsibility |
| --- | --- |
| `lib.rs` | capabilities, the initialize handshake, the request/notification loop |
| `analysis.rs` | run the front-end; shape errors/warnings into diagnostics; cache the last good analysis |
| `document.rs` | the set of open documents and their latest analysis |
| `line_index.rs` | byte offset ↔ LSP `Position` (UTF-16) |
| `completion.rs` | completion candidates (fields, keywords, top-level names) |
| `hover.rs` | the type under the cursor |

## Build

```sh
cargo build --release -p wraith-lsp   # from the repo root
# binary: target/release/wraith-lsp
```

## Test

```sh
cargo test -p wraith-lsp
```

`tests/server.rs` drives the real `serve` loop over an in-memory connection
through the real initialize handshake and the real JSON-RPC message types — the
same path VS Code exercises — and asserts diagnostics, field completion, general
completion, hover, and completion surviving a broken buffer.

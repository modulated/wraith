// VS Code client for the Wraith language server.
//
// This is deliberately thin: it locates the `wraith-lsp` binary and hands the
// whole conversation to it over stdio via vscode-languageclient. All the
// language intelligence — diagnostics, completion, hover — lives in the Rust
// server, which reuses the compiler. Syntax highlighting is separate (the
// TextMate grammar in wraith.tmLanguage.json) and keeps working even when the
// server is disabled or missing.

import { workspace, window, ExtensionContext } from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

export function activate(context: ExtensionContext) {
  const config = workspace.getConfiguration("wraith");
  if (!config.get<boolean>("server.enable", true)) {
    return;
  }

  const command = config.get<string>("server.path", "wraith-lsp") || "wraith-lsp";

  // One server process, talking LSP over stdio. TransportKind.stdio matches the
  // server's `Connection::stdio()`.
  const serverOptions: ServerOptions = {
    run: { command, transport: TransportKind.stdio },
    debug: { command, transport: TransportKind.stdio },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "wraith" }],
    synchronize: {
      fileEvents: workspace.createFileSystemWatcher("**/*.wr"),
    },
  };

  client = new LanguageClient(
    "wraith",
    "Wraith Language Server",
    serverOptions,
    clientOptions,
  );

  client.start().catch((err) => {
    window.showErrorMessage(
      `Wraith language server failed to start (command: "${command}"). ` +
        `Build it with \`cargo build --release -p wraith-lsp\` and set ` +
        `"wraith.server.path", or add the binary to your PATH. (${err})`,
    );
  });

  context.subscriptions.push({
    dispose: () => {
      void client?.stop();
    },
  });
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}

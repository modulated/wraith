//! End-to-end test of the server over an in-memory LSP connection.
//!
//! This drives the real `serve` loop through the real initialize handshake and
//! the real JSON-RPC message types — the same path VS Code exercises — and
//! checks that diagnostics, completion, and hover come back correctly. It is the
//! proof that the client↔server↔compiler loop is wired, not just that it
//! compiles.

use std::str::FromStr;
use std::thread;

use lsp_server::{Connection, Message, Notification, Request, RequestId};
use lsp_types::{
    CompletionParams, CompletionResponse, Diagnostic, DiagnosticSeverity, Hover, HoverParams,
    InitializeParams, Position, TextDocumentContentChangeEvent, TextDocumentIdentifier,
    TextDocumentItem, TextDocumentPositionParams, Uri, VersionedTextDocumentIdentifier,
};
use serde_json::json;

fn uri() -> Uri {
    Uri::from_str("file:///test.wr").unwrap()
}

/// Position of the byte offset just past the first occurrence of `needle`.
fn pos_after(src: &str, needle: &str) -> Position {
    let idx = src.find(needle).unwrap() + needle.len();
    let prefix = &src[..idx];
    let line = prefix.matches('\n').count() as u32;
    let col = prefix.len() - prefix.rfind('\n').map(|i| i + 1).unwrap_or(0);
    Position {
        line,
        character: col as u32,
    }
}

struct Client {
    conn: Connection,
    next_id: i32,
}

impl Client {
    fn request<P: serde::Serialize>(&mut self, method: &str, params: P) -> RequestId {
        let id = RequestId::from(self.next_id);
        self.next_id += 1;
        self.conn
            .sender
            .send(Message::Request(Request {
                id: id.clone(),
                method: method.to_string(),
                params: serde_json::to_value(params).unwrap(),
            }))
            .unwrap();
        id
    }

    fn notify<P: serde::Serialize>(&self, method: &str, params: P) {
        self.conn
            .sender
            .send(Message::Notification(Notification {
                method: method.to_string(),
                params: serde_json::to_value(params).unwrap(),
            }))
            .unwrap();
    }

    /// Read messages until the response with `id` arrives, returning its result.
    fn response(&self, id: RequestId) -> serde_json::Value {
        loop {
            match self.conn.receiver.recv().unwrap() {
                Message::Response(r) if r.id == id => return r.result.unwrap_or(json!(null)),
                _ => {} // ignore interleaved notifications / other responses
            }
        }
    }

    /// Read messages until a publishDiagnostics notification arrives.
    fn diagnostics(&self) -> Vec<Diagnostic> {
        loop {
            if let Message::Notification(n) = self.conn.receiver.recv().unwrap()
                && n.method == "textDocument/publishDiagnostics"
            {
                let v = n.params.get("diagnostics").cloned().unwrap();
                return serde_json::from_value(v).unwrap();
            }
        }
    }
}

const PROGRAM: &str = "\
struct Entry { start: u16, flags: u8 }
fn read_flags(e: Entry) -> u8 {
    return e.flags;
}
#[reset]
fn main() {
    let x: Entry = Entry { start: 4, flags: 7 };
    loop {}
}
";

fn setup() -> (Client, thread::JoinHandle<()>) {
    let (server, client) = Connection::memory();
    let handle = thread::spawn(move || {
        wraith_lsp::serve(server).unwrap();
    });
    let mut client = Client {
        conn: client,
        next_id: 1,
    };
    // Initialize handshake.
    let id = client.request("initialize", InitializeParams::default());
    let _ = client.response(id);
    client.notify("initialized", json!({}));
    (client, handle)
}

fn open(client: &Client, text: &str) {
    client.notify(
        "textDocument/didOpen",
        json!({
            "textDocument": TextDocumentItem {
                uri: uri(),
                language_id: "wraith".to_string(),
                version: 0,
                text: text.to_string(),
            }
        }),
    );
}

fn shutdown(client: &mut Client, handle: thread::JoinHandle<()>) {
    let id = client.request("shutdown", json!(null));
    let _ = client.response(id);
    client.notify("exit", json!(null));
    drop(client.conn.sender.clone());
    let _ = handle.join();
}

#[test]
fn a_clean_program_reports_no_errors() {
    let (mut client, handle) = setup();
    open(&client, PROGRAM);
    let diags = client.diagnostics();
    assert!(
        diags
            .iter()
            .all(|d| d.severity != Some(DiagnosticSeverity::ERROR)),
        "a valid program should have no error diagnostics: {diags:?}"
    );
    shutdown(&mut client, handle);
}

#[test]
fn a_type_error_is_reported_with_a_range() {
    let (mut client, handle) = setup();
    // `flags` is u8; assigning a struct to it is a type error.
    let broken = PROGRAM.replace("flags: 7", "flags: x");
    open(&client, &broken);
    let diags = client.diagnostics();
    let errors: Vec<_> = diags
        .iter()
        .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
        .collect();
    assert_eq!(errors.len(), 1, "expected one error, got {diags:?}");
    // The range must be non-empty and not the ` at N..M` noise in the message.
    assert!(
        !errors[0].message.contains(".."),
        "offsets leaked into message: {:?}",
        errors[0].message
    );
    shutdown(&mut client, handle);
}

#[test]
fn completing_after_a_dot_lists_struct_fields() {
    let (mut client, handle) = setup();
    open(&client, PROGRAM);
    let _ = client.diagnostics(); // drain the open's publish

    // Cursor right after the `.` in `e.flags`.
    let position = pos_after(PROGRAM, "return e.");
    let id = client.request(
        "textDocument/completion",
        CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri() },
                position,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: None,
        },
    );
    let resp: Option<CompletionResponse> = serde_json::from_value(client.response(id)).unwrap();
    let labels = labels(resp);
    assert!(
        labels.contains(&"flags".to_string()),
        "fields missing: {labels:?}"
    );
    assert!(
        labels.contains(&"start".to_string()),
        "fields missing: {labels:?}"
    );
    // Field completion must NOT dump keywords.
    assert!(
        !labels.contains(&"fn".to_string()),
        "leaked keywords: {labels:?}"
    );
    shutdown(&mut client, handle);
}

#[test]
fn general_completion_offers_keywords_and_top_level_names() {
    let (mut client, handle) = setup();
    open(&client, PROGRAM);
    let _ = client.diagnostics();

    // A blank position inside main's body (not after a dot).
    let position = pos_after(
        PROGRAM,
        "let x: Entry = Entry { start: 4, flags: 7 };\n    ",
    );
    let id = client.request(
        "textDocument/completion",
        CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri() },
                position,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: None,
        },
    );
    let labels = labels(serde_json::from_value(client.response(id)).unwrap());
    assert!(
        labels.contains(&"fn".to_string()),
        "keyword missing: {labels:?}"
    );
    assert!(
        labels.contains(&"read_flags".to_string()),
        "top-level fn missing: {labels:?}"
    );
    assert!(
        labels.contains(&"Entry".to_string()),
        "struct missing: {labels:?}"
    );
    assert!(
        labels.contains(&"u8".to_string()),
        "primitive type missing: {labels:?}"
    );
    shutdown(&mut client, handle);
}

#[test]
fn hovering_a_typed_expression_shows_its_type() {
    let (mut client, handle) = setup();
    open(&client, PROGRAM);
    let _ = client.diagnostics();

    // Hover the `e` reference in `return e.flags`, whose type is Entry. (A
    // binding site like `let x` is not an expression and carries no type.)
    let position = pos_after(PROGRAM, "return ");
    let id = client.request(
        "textDocument/hover",
        HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri() },
                position,
            },
            work_done_progress_params: Default::default(),
        },
    );
    let hover: Option<Hover> = serde_json::from_value(client.response(id)).unwrap();
    let text = hover_text(hover);
    assert!(
        text.contains("Entry"),
        "hover should name the type, got: {text:?}"
    );
    shutdown(&mut client, handle);
}

#[test]
fn completion_survives_a_broken_buffer_via_the_last_good_analysis() {
    let (mut client, handle) = setup();
    open(&client, PROGRAM);
    let _ = client.diagnostics();

    // Edit to an unparseable buffer: a trailing `e.` with nothing after it.
    let broken = PROGRAM.replace("return e.flags;", "return e.");
    client.notify(
        "textDocument/didChange",
        lsp_types::DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri(),
                version: 1,
            },
            content_changes: vec![TextDocumentContentChangeEvent {
                range: None,
                range_length: None,
                text: broken.clone(),
            }],
        },
    );
    let _ = client.diagnostics(); // the parse error

    // Completion at the broken `e.` must still list fields, from the retained
    // good analysis.
    let position = pos_after(&broken, "return e.");
    let id = client.request(
        "textDocument/completion",
        CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri() },
                position,
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: None,
        },
    );
    let labels = labels(serde_json::from_value(client.response(id)).unwrap());
    assert!(
        labels.contains(&"flags".to_string()),
        "field completion should survive a broken buffer: {labels:?}"
    );
    shutdown(&mut client, handle);
}

fn labels(resp: Option<CompletionResponse>) -> Vec<String> {
    match resp {
        Some(CompletionResponse::Array(items)) => items.into_iter().map(|i| i.label).collect(),
        Some(CompletionResponse::List(list)) => list.items.into_iter().map(|i| i.label).collect(),
        None => Vec::new(),
    }
}

fn hover_text(hover: Option<Hover>) -> String {
    use lsp_types::{HoverContents, MarkedString};
    match hover.map(|h| h.contents) {
        Some(HoverContents::Markup(m)) => m.value,
        Some(HoverContents::Scalar(MarkedString::String(s))) => s,
        Some(HoverContents::Scalar(MarkedString::LanguageString(s))) => s.value,
        Some(HoverContents::Array(v)) => format!("{v:?}"),
        None => String::new(),
    }
}

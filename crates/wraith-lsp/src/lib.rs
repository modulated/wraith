//! Wraith language server.
//!
//! A synchronous LSP server built on `lsp-server`, reusing the Wraith compiler
//! as a library. There is no async runtime and no incremental machinery: on
//! each edit the whole document is re-lexed, re-parsed and re-analyzed. That is
//! affordable because the front-end runs in single-digit milliseconds on the
//! file sizes this language sees (a few hundred to a couple thousand lines), and
//! it keeps the server simple. The editor is expected to debounce changes.

mod analysis;
mod completion;
mod document;
mod hover;
mod line_index;

use lsp_server::{Connection, ExtractError, Message, Notification, Request, RequestId, Response};
use lsp_types::{
    CompletionOptions, CompletionParams, DidChangeTextDocumentParams, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, HoverParams, HoverProviderCapability, PublishDiagnosticsParams,
    ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, Uri,
};

use document::Documents;

pub type Error = Box<dyn std::error::Error + Sync + Send>;

/// Declare capabilities, complete the LSP initialize handshake, and serve
/// requests until the client shuts down.
///
/// Takes the connection **by value** and drops it on return. That drop is
/// load-bearing for the stdio transport: it closes the sender the stdout writer
/// thread reads from, so the caller's `io_threads.join()` can finish. Hold the
/// connection past this call and the join deadlocks.
pub fn serve(connection: Connection) -> Result<(), Error> {
    let capabilities = ServerCapabilities {
        // Full sync: the client sends the whole document text on each change.
        // Simplest correct option; incremental sync is an optimization we do not
        // need at these file sizes.
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        completion_provider: Some(CompletionOptions {
            // `.` re-triggers completion so field lists appear as you type it.
            trigger_characters: Some(vec![".".to_string()]),
            ..Default::default()
        }),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        ..Default::default()
    };
    let capabilities = serde_json::to_value(capabilities)?;
    let _init = connection.initialize(capabilities)?;

    let mut docs = Documents::default();
    main_loop(&connection, &mut docs)
    // `connection` drops here.
}

fn main_loop(connection: &Connection, docs: &mut Documents) -> Result<(), Error> {
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                handle_request(connection, docs, req)?;
            }
            Message::Notification(note) => handle_notification(connection, docs, note)?,
            Message::Response(_) => {}
        }
    }
    Ok(())
}

fn handle_request(
    connection: &Connection,
    docs: &mut Documents,
    req: Request,
) -> Result<(), Error> {
    use lsp_types::request::{Completion, HoverRequest, Request as _};

    match req.method.as_str() {
        Completion::METHOD => {
            let (id, params) = cast_req::<Completion>(req)?;
            let items = completion_response(docs, params);
            respond(connection, id, items)?;
        }
        HoverRequest::METHOD => {
            let (id, params) = cast_req::<HoverRequest>(req)?;
            let result = hover_response(docs, params);
            respond(connection, id, result)?;
        }
        _ => {
            // Unknown request: reply with null so the client is not left waiting.
            respond(connection, req.id, serde_json::Value::Null)?;
        }
    }
    Ok(())
}

fn completion_response(
    docs: &Documents,
    params: CompletionParams,
) -> Vec<lsp_types::CompletionItem> {
    let pos = params.text_document_position;
    let Some(doc) = docs.get(&pos.text_document.uri) else {
        return Vec::new();
    };
    let offset = doc.index.offset(&doc.text, pos.position);
    completion::complete(doc, offset)
}

fn hover_response(docs: &Documents, params: HoverParams) -> Option<lsp_types::Hover> {
    let pos = params.text_document_position_params;
    let doc = docs.get(&pos.text_document.uri)?;
    let offset = doc.index.offset(&doc.text, pos.position);
    hover::hover(doc, offset)
}

fn handle_notification(
    connection: &Connection,
    docs: &mut Documents,
    note: Notification,
) -> Result<(), Error> {
    use lsp_types::notification::{
        DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
    };

    match note.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let params: DidOpenTextDocumentParams = cast_note(note)?;
            let uri = params.text_document.uri;
            let diags = docs.set(&uri, params.text_document.text);
            publish(connection, uri, diags)?;
        }
        DidChangeTextDocument::METHOD => {
            let params: DidChangeTextDocumentParams = cast_note(note)?;
            let uri = params.text_document.uri;
            // Full sync: the last content change carries the entire new text.
            if let Some(change) = params.content_changes.into_iter().last() {
                let diags = docs.set(&uri, change.text);
                publish(connection, uri, diags)?;
            }
        }
        DidCloseTextDocument::METHOD => {
            let params: DidCloseTextDocumentParams = cast_note(note)?;
            let uri = params.text_document.uri;
            docs.remove(&uri);
            // Clear the editor's squiggles for a file we no longer track.
            publish(connection, uri, Vec::new())?;
        }
        _ => {}
    }
    Ok(())
}

fn publish(
    connection: &Connection,
    uri: Uri,
    diagnostics: Vec<lsp_types::Diagnostic>,
) -> Result<(), Error> {
    use lsp_types::notification::{Notification as _, PublishDiagnostics};
    let params = PublishDiagnosticsParams {
        uri,
        diagnostics,
        version: None,
    };
    connection.sender.send(Message::Notification(Notification {
        method: PublishDiagnostics::METHOD.to_string(),
        params: serde_json::to_value(params)?,
    }))?;
    Ok(())
}

fn respond<T: serde::Serialize>(
    connection: &Connection,
    id: RequestId,
    result: T,
) -> Result<(), Error> {
    let resp = Response {
        id,
        result: Some(serde_json::to_value(result)?),
        error: None,
    };
    connection.sender.send(Message::Response(resp))?;
    Ok(())
}

fn cast_req<R>(req: Request) -> Result<(RequestId, R::Params), Error>
where
    R: lsp_types::request::Request,
    R::Params: serde::de::DeserializeOwned,
{
    req.extract(R::METHOD).map_err(unwrap_extract)
}

fn cast_note<P>(note: Notification) -> Result<P, Error>
where
    P: serde::de::DeserializeOwned,
{
    let method = note.method.clone();
    note.extract::<P>(&method).map_err(unwrap_extract)
}

fn unwrap_extract<T>(e: ExtractError<T>) -> Error {
    match e {
        ExtractError::JsonError { method, error } => {
            format!("malformed params for {method}: {error}").into()
        }
        ExtractError::MethodMismatch(_) => "internal: method mismatch".into(),
    }
}

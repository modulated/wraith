//! The `wraith-lsp` binary: wire the language server to stdio.
//!
//! All logic lives in the library (`wraith_lsp::serve`) so it can be driven over
//! an in-memory connection in tests. This entry point only owns the transport.

fn main() -> Result<(), wraith_lsp::Error> {
    let (connection, io_threads) = lsp_server::Connection::stdio();
    // `serve` takes the connection by value and drops it, which lets the writer
    // thread finish so this join can return.
    wraith_lsp::serve(connection)?;
    io_threads.join()?;
    Ok(())
}

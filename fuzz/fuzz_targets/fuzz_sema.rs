use afl::fuzz;

fn main() {
    fuzz!(|data: &[u8]| {
        // Convert bytes to string
        if let Ok(input) = std::str::from_utf8(data) {
            // Lex the input
            if let Ok(tokens) = wraith::lex(input) {
                // Parse the tokens
                if let Ok(ast) = wraith::Parser::parse(&tokens) {
                    // The phases past parsing are where the panics hide:
                    // const evaluation (a constant string slice at a
                    // non-UTF-8 boundary panicked the compiler), type
                    // resolution, frame layout. Drive all of sema.
                    let _ = wraith::sema::analyze(&ast);
                }
            }
        }
    });
}

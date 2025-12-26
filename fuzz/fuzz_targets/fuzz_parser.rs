use afl::fuzz;

fn main() {
    fuzz!(|data: &[u8]| {
        // Convert bytes to string
        if let Ok(input) = std::str::from_utf8(data) {
            // Lex the input
            if let Ok(tokens) = wraith::lex(input) {
                // Parse the tokens
                let _ = wraith::Parser::parse(&tokens);
            }
        }
    });
}

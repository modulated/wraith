use afl::fuzz;

fn main() {
    fuzz!(|data: &[u8]| {
        // Convert bytes to string
        if let Ok(input) = std::str::from_utf8(data) {
            // Lex the input
            if let Ok(tokens) = wraith::lex(input) {
                // Parse the tokens
                if let Ok(ast) = wraith::Parser::parse(&tokens) {
                    // Drive all of sema.
                    if let Ok(prog_info) = wraith::sema::analyze(&ast) {
                        // Code gen. Target the compiler's default CPU, as the CLI does.
                        let _ = wraith::codegen::generate(
                            &ast,
                            &prog_info,
                            wraith::codegen::CommentVerbosity::Normal,
                            wraith::codegen::TargetCpu::default(),
                        );
                    }
                }
            }
        }
    });
}

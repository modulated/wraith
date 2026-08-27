# Fuzzing Wraith with AFL++

This directory contains fuzzing harnesses for testing Wraith with AFL++ (American Fuzzy Lop).

## Quick Start

### 1. Install Dependencies

**macOS:**

```bash
brew install afl++
cargo install cargo-afl
```

**Linux:**

```bash
sudo apt install afl++
cargo install cargo-afl
```

### 2. Build the Fuzz Target

```bash
cd fuzz
cargo afl build --release
```

**Note:** The first build takes ~2-3 minutes as it compiles the fuzzer with AFL instrumentation. This is normal!

### 3. Run the Fuzzer

```bash
# Create output directory
mkdir -p out

# Run AFL++ with seed inputs
cargo afl fuzz -i seeds -o out target/release/fuzz_full_stack
```

### 4. Monitor Progress

AFL++ will show a live dashboard with:

- **total paths**: Unique execution paths discovered
- **crashes**: Number of crashes found
- **hangs**: Number of hangs detected
- **execs/sec**: Fuzzing speed

Press `Ctrl+C` to stop fuzzing.

## Viewing Results

### Crashes

```bash
ls -la out/default/crashes/
```

### Reproduce a crash

```bash
cat out/default/crashes/id:000000* | cargo run --
```

### Minimize a crashing input

```bash
cargo afl tmin -i out/default/crashes/id:000000* -o minimized.wr -- target/release/fuzz_full_stack
```

## Advanced Usage

### Multiple Cores

Run AFL++ in parallel for faster fuzzing:

```bash
# Terminal 1 (master)
cargo afl fuzz -i seeds -o out -M fuzzer1 target/release/fuzz_full_stack

# Terminal 2 (secondary)
cargo afl fuzz -i seeds -o out -S fuzzer2 target/release/fuzz_full_stack

# Terminal 3 (secondary)
cargo afl fuzz -i seeds -o out -S fuzzer3 target/release/fuzz_full_stack
```

### Persistent Mode (Faster)

For even better performance, use AFL++'s persistent mode:

```rust
use afl::fuzz;

fn main() {
    fuzz!(|data: &[u8]| {
        if let Ok(input) = std::str::from_utf8(data) {
            let tokens = wraith::lex(input);
            let _ = wraith::Parser::parse(&tokens);
        }
    });
}
```

This is already enabled in our fuzz target!

### Custom Dictionary

Create `fuzz/dict.txt` with Wraith keywords to guide fuzzing.
Then run with:

```bash
cargo afl fuzz -i seeds -o out -x dict.txt target/release/fuzz_full_stack
```

## Targets

### fuzz_full_stack

- runs lexer, parser, sema, codegen in sequence

## Tips

1. **Run overnight**: Fuzzing is most effective over long periods (hours/days)
2. **Use multiple cores**: AFL++ scales well with parallel fuzzing
3. **Monitor memory**: Watch for memory leaks with `top` or `htop`
4. **Save corpus**: The `out/` directory contains valuable test cases
5. **Integrate with CI**: Run fuzzing in CI to catch regressions

## Sanitizers

Combine with sanitizers for better bug detection:

### AddressSanitizer

```bash
RUSTFLAGS="-Z sanitizer=address" cargo afl build --release
```

### UndefinedBehaviorSanitizer

```bash
RUSTFLAGS="-Z sanitizer=undefined" cargo afl build --release
```

## Resources

- [AFL++ Documentation](https://github.com/AFLplusplus/AFLplusplus)
- [cargo-afl](https://github.com/rust-fuzz/afl.rs)
- [Rust Fuzz Book](https://rust-fuzz.github.io/book/)

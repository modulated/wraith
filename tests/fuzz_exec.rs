//! Execution-checked fuzzing: generate random programs, run them, and check the
//! answers.
//!
//! The AFL target in `fuzz/` drives lex → parse → sema → codegen and discards
//! the result, so it proves the compiler does not *crash*. It cannot prove the
//! compiler does not *lie*. Every miscompile found in this codebase so far —
//! `match` ranges comparing one byte of a `u16`, a struct argument staging half
//! a pointer, X clobbered across a recursive call, an array field storing a
//! pointer instead of its elements — compiled cleanly and returned a wrong
//! answer.
//!
//! This closes that gap with two independent checks on every generated program:
//!
//! 1. **An oracle.** The generator builds an expression tree, so evaluating it
//!    in Rust is easy, and the emitted program must produce that value.
//! 2. **Metamorphic agreement.** The same expression is emitted in several
//!    surface forms — inline, returned from a function, inside a `match` arm,
//!    inside a single-iteration loop — which must all agree. This catches the
//!    case the oracle cannot: a misunderstanding shared by the generator and
//!    the compiler, where one *form* still diverges.
//!
//! Runs are deterministic. Each iteration is seeded from its index, so a
//! failure reports a seed that reproduces it, and CI sees the same programs
//! every time. Crank it up locally:
//!
//! ```text
//! WRAITH_FUZZ_ITERS=20000 cargo test --test fuzz_exec -- --nocapture
//! WRAITH_FUZZ_SEED=12345  cargo test --test fuzz_exec -- --nocapture
//! ```
//!
//! Scope, deliberately narrow so the oracle is certainly right — a false
//! positive here costs more than a missed bug. `u8`/`u16` only (signed
//! division and arithmetic shift are their own semantics, not yet mirrored);
//! divisors are nonzero literals, sidestepping division-by-zero, which is an
//! error-behaviour question rather than an arithmetic one; expressions are
//! fully parenthesised, so a precedence disagreement cannot masquerade as a
//! codegen bug. Widening any of these means extending the oracle to match.

#[path = "common/mod.rs"]
mod common;

use common::exec::run;

// ---------------------------------------------------------------------------
// Deterministic PRNG (SplitMix64) — no dependency, and reproducible by seed.
// ---------------------------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(1))
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in `0..n`.
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

// ---------------------------------------------------------------------------
// A small expression language the oracle can evaluate exactly
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Width {
    U8,
    U16,
}

impl Width {
    fn mask(self) -> u32 {
        match self {
            Width::U8 => 0xFF,
            Width::U16 => 0xFFFF,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Width::U8 => "u8",
            Width::U16 => "u16",
        }
    }
    /// Shift counts at or above the width produce zero, so keep them below it.
    fn shift_limit(self) -> u32 {
        match self {
            Width::U8 => 8,
            Width::U16 => 16,
        }
    }
}

#[derive(Clone, Copy)]
enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    And,
    Or,
    Xor,
    Shl,
    Shr,
}

impl Op {
    fn sym(self) -> &'static str {
        match self {
            Op::Add => "+",
            Op::Sub => "-",
            Op::Mul => "*",
            Op::Div => "/",
            Op::Rem => "%",
            Op::And => "&",
            Op::Or => "|",
            Op::Xor => "^",
            Op::Shl => "<<",
            Op::Shr => ">>",
        }
    }
}

enum Expr {
    Lit(u32),
    Var(usize),
    Bin(Box<Expr>, Op, Box<Expr>),
}

const VARS: usize = 4;

/// Build an expression of at most `depth` further levels.
fn gen_expr(rng: &mut Rng, w: Width, depth: u32) -> Expr {
    if depth == 0 || rng.below(100) < 30 {
        return if rng.below(2) == 0 {
            Expr::Lit(rng.next() as u32 & w.mask())
        } else {
            Expr::Var(rng.below(VARS as u64) as usize)
        };
    }

    let op = match rng.below(10) {
        0 => Op::Add,
        1 => Op::Sub,
        2 => Op::Mul,
        3 => Op::Div,
        4 => Op::Rem,
        5 => Op::And,
        6 => Op::Or,
        7 => Op::Xor,
        8 => Op::Shl,
        _ => Op::Shr,
    };

    let lhs = gen_expr(rng, w, depth - 1);
    let rhs = match op {
        // A zero divisor is an error-behaviour question, not an arithmetic one:
        // keep it out so the oracle stays unarguable.
        Op::Div | Op::Rem => Expr::Lit(1 + (rng.next() as u32 & (w.mask() >> 1))),
        // At or past the width the result is zero; that is worth testing, but
        // separately from the arithmetic, so stay inside the useful range.
        Op::Shl | Op::Shr => Expr::Lit(rng.below(w.shift_limit() as u64) as u32),
        _ => gen_expr(rng, w, depth - 1),
    };
    Expr::Bin(Box::new(lhs), op, Box::new(rhs))
}

/// The value the program must produce. Wrapping at the type's width throughout,
/// matching the language ("all arithmetic operators wrap on overflow").
fn eval(e: &Expr, vars: &[u32; VARS], w: Width) -> u32 {
    let m = w.mask();
    match e {
        Expr::Lit(v) => v & m,
        Expr::Var(i) => vars[*i] & m,
        Expr::Bin(l, op, r) => {
            let a = eval(l, vars, w);
            let b = eval(r, vars, w);
            let v = match op {
                Op::Add => a.wrapping_add(b),
                Op::Sub => a.wrapping_sub(b),
                Op::Mul => a.wrapping_mul(b),
                Op::Div => a / b.max(1),
                Op::Rem => a % b.max(1),
                Op::And => a & b,
                Op::Or => a | b,
                Op::Xor => a ^ b,
                Op::Shl => {
                    if b >= w.shift_limit() {
                        0
                    } else {
                        a << b
                    }
                }
                Op::Shr => {
                    if b >= w.shift_limit() {
                        0
                    } else {
                        (a & m) >> b
                    }
                }
            };
            v & m
        }
    }
}

/// Fully parenthesised, so the parser's precedence cannot make the oracle wrong.
fn render(e: &Expr) -> String {
    match e {
        Expr::Lit(v) => format!("{v}"),
        Expr::Var(i) => format!("v{i}"),
        Expr::Bin(l, op, r) => format!("({} {} {})", render(l), op.sym(), render(r)),
    }
}

// ---------------------------------------------------------------------------
// Emitting the same computation several ways
// ---------------------------------------------------------------------------

/// The surface forms one expression is written in. All must agree; each routes
/// through a different part of codegen.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Form {
    /// Assigned straight to the output address.
    Inline,
    /// Computed in a called function and returned.
    ViaFunction,
    /// Computed inside the taken arm of a `match`.
    ViaMatch,
    /// Computed inside a loop that runs exactly once.
    ViaLoop,
}

const FORMS: [Form; 4] = [
    Form::Inline,
    Form::ViaFunction,
    Form::ViaMatch,
    Form::ViaLoop,
];

fn program(e: &Expr, vars: &[u32; VARS], w: Width, form: Form) -> String {
    let ty = w.name();
    let decls: String = (0..VARS)
        .map(|i| format!("    let v{i}: {ty} = {};\n", vars[i]))
        .collect();
    let body = render(e);

    // A u16 result is written out as two bytes; a u8 as one.
    let store = |val: &str| -> String {
        match w {
            Width::U8 => format!("    OUT = {val};\n"),
            Width::U16 => {
                format!("    let r: u16 = {val};\n    OUT = r.low;\n    OUT1 = r.high;\n")
            }
        }
    };

    let head = "const OUT: addr = 0x0900;\nconst OUT1: addr = 0x0901;\n";

    match form {
        Form::Inline => format!(
            "{head}#[reset]\nfn main() {{\n{decls}{}    loop {{}}\n}}\n",
            store(&body)
        ),
        Form::ViaFunction => format!(
            "{head}fn compute() -> {ty} {{\n{decls}    return {body};\n}}\n\
             #[reset]\nfn main() {{\n{}    loop {{}}\n}}\n",
            store("compute()")
        ),
        Form::ViaMatch => format!(
            "{head}#[reset]\nfn main() {{\n{decls}    let sel: u8 = 0;\n\
                 match sel {{\n        0 => {{\n{}        }}\n        _ => {{ OUT = 0; }}\n    }}\n\
                 loop {{}}\n}}\n",
            store(&body)
                .lines()
                .map(|l| format!("    {l}\n"))
                .collect::<String>()
        ),
        Form::ViaLoop => format!(
            "{head}#[reset]\nfn main() {{\n{decls}    for i in 0..1 {{\n{}    }}\n    loop {{}}\n}}\n",
            store(&body)
                .lines()
                .map(|l| format!("    {l}\n"))
                .collect::<String>()
        ),
    }
}

/// Compile and run one program, returning the value at the output address, or
/// the reason it did not get there.
fn observe(src: &str, w: Width) -> Result<u32, String> {
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut e = run(src);
        match w {
            Width::U8 => e.mem(0x0900) as u32,
            Width::U16 => e.mem16(0x0900) as u32,
        }
    }));
    res.map_err(|p| {
        p.downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_else(|| "<panic>".into())
            .lines()
            .take(3)
            .collect::<Vec<_>>()
            .join(" | ")
    })
}

fn iterations() -> u64 {
    std::env::var("WRAITH_FUZZ_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(150)
}

/// One generated case, checked against the oracle in every surface form.
fn check_seed(seed: u64) -> Result<(), String> {
    let mut rng = Rng::new(seed);
    let w = if rng.below(2) == 0 {
        Width::U8
    } else {
        Width::U16
    };
    let mut vars = [0u32; VARS];
    for v in vars.iter_mut() {
        *v = rng.next() as u32 & w.mask();
    }
    let e = gen_expr(&mut rng, w, 4);
    let expected = eval(&e, &vars, w);

    for form in FORMS {
        let src = program(&e, &vars, w, form);
        match observe(&src, w) {
            Ok(got) if got == expected => {}
            Ok(got) => {
                return Err(format!(
                    "seed {seed}: {:?} form gave {got}, expected {expected}\n\
                     expression: {}\nvars: {vars:?}\n--- source ---\n{src}",
                    form,
                    render(&e)
                ));
            }
            Err(why) => {
                return Err(format!(
                    "seed {seed}: {:?} form failed to run: {why}\n\
                     expression: {}\nvars: {vars:?}\n--- source ---\n{src}",
                    form,
                    render(&e)
                ));
            }
        }
    }
    Ok(())
}

#[test]
fn generated_programs_compute_what_they_should() {
    // Quiet the default hook: a rejected program is reported by `observe`, and
    // the backtrace spam would bury the failing source.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));

    let iters = iterations();
    let base: u64 = std::env::var("WRAITH_FUZZ_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);

    let mut failures = Vec::new();
    for i in 0..iters {
        if let Err(e) = check_seed(base.wrapping_add(i)) {
            failures.push(e);
            // A handful of examples is enough to debug; do not print hundreds.
            if failures.len() >= 3 {
                break;
            }
        }
    }

    std::panic::set_hook(prev);
    assert!(
        failures.is_empty(),
        "{} of {iters} generated programs were miscompiled:\n\n{}",
        failures.len(),
        failures.join("\n\n========================\n\n")
    );
}

/// The generator itself must be reproducible, or a reported seed is useless.
#[test]
fn a_seed_reproduces_its_program() {
    let build = |seed: u64| {
        let mut rng = Rng::new(seed);
        let w = if rng.below(2) == 0 {
            Width::U8
        } else {
            Width::U16
        };
        let mut vars = [0u32; VARS];
        for v in vars.iter_mut() {
            *v = rng.next() as u32 & w.mask();
        }
        let e = gen_expr(&mut rng, w, 4);
        (render(&e), vars, w.name())
    };
    for seed in [0u64, 1, 42, 9999] {
        assert_eq!(
            build(seed),
            build(seed),
            "seed {seed} produced two different programs"
        );
    }
}

/// The oracle has to be exercised by the shapes it claims to cover, or a
/// generator change could quietly stop producing them.
#[test]
fn the_generator_covers_its_operators() {
    let mut seen = std::collections::HashSet::new();
    fn walk(e: &Expr, seen: &mut std::collections::HashSet<&'static str>) {
        if let Expr::Bin(l, op, r) = e {
            seen.insert(op.sym());
            walk(l, seen);
            walk(r, seen);
        }
    }
    for seed in 0..400u64 {
        let mut rng = Rng::new(seed);
        let w = if rng.below(2) == 0 {
            Width::U8
        } else {
            Width::U16
        };
        for _ in 0..VARS {
            rng.next();
        }
        walk(&gen_expr(&mut rng, w, 4), &mut seen);
    }
    for op in ["+", "-", "*", "/", "%", "&", "|", "^", "<<", ">>"] {
        assert!(seen.contains(op), "generator never emitted `{op}`");
    }
}

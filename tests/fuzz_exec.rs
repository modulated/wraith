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
//! 1. **An oracle.** The generator builds the program as a tree, so running it
//!    in Rust is exact, and the emitted program must reach the same final state.
//! 2. **Metamorphic agreement.** The same program is emitted in several surface
//!    forms — inline, inside a called function, inside a `match` arm, inside a
//!    single-iteration loop — which must all agree. This catches the case the
//!    oracle cannot: a misunderstanding shared by the generator and the
//!    compiler, where one *form* still diverges.
//!
//! What is generated is a small imperative language: four integer types, ten
//! binary operators, casts, comparisons and boolean connectives, assignment,
//! `if`/`else`, counted `for` and condition-driven `while`, nested. Every
//! variable's final value is written out, so one program checks four results
//! rather than one.
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
//! Everything the generator emits is pinned by the language spec, because an
//! oracle that is merely *probably* right is worse than no oracle — a false
//! positive here costs more than a missed bug. The remaining restrictions are
//! all of that kind, and each is a deliberate exclusion rather than an
//! oversight:
//!
//! - **Divisors are nonzero positive literals.** Division by zero is an
//!   error-behaviour question, not an arithmetic one; a positive divisor also
//!   keeps `i8::MIN / -1` (the one signed division that overflows) out.
//! - **Shift counts stay below the width**, where the result is the plain
//!   shift. At or past the width is worth testing, but as its own question.
//! - **Expressions are fully parenthesised**, so a precedence disagreement
//!   cannot masquerade as a codegen bug.
//! - **One type per program.** Mixed-width arithmetic brings the implicit
//!   widening rules into the oracle; casts are exercised instead by casting out
//!   to another type and straight back, which is exactly truncation and
//!   sign-extension with nothing else attached.
//! - **`while` loops count down a dedicated variable** that no generated
//!   assignment can touch, so termination is a property of the generator rather
//!   than a hope.
//! - **Every operator has an operand that mentions a variable.** An expression
//!   of pure literals is typed by the language from the literals' own range,
//!   not from the program around it, so it is not evaluated at the program's
//!   width — a different question, and one the oracle does not model.

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
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Ty {
    U8,
    U16,
    I8,
    I16,
}

const TYPES: [Ty; 4] = [Ty::U8, Ty::U16, Ty::I8, Ty::I16];

impl Ty {
    fn name(self) -> &'static str {
        match self {
            Ty::U8 => "u8",
            Ty::U16 => "u16",
            Ty::I8 => "i8",
            Ty::I16 => "i16",
        }
    }
    fn bits(self) -> u32 {
        match self {
            Ty::U8 | Ty::I8 => 8,
            Ty::U16 | Ty::I16 => 16,
        }
    }
    fn signed(self) -> bool {
        matches!(self, Ty::I8 | Ty::I16)
    }
    fn wide(self) -> bool {
        self.bits() == 16
    }
    fn min(self) -> i64 {
        if self.signed() {
            -(1i64 << (self.bits() - 1))
        } else {
            0
        }
    }
    fn max(self) -> i64 {
        if self.signed() {
            (1i64 << (self.bits() - 1)) - 1
        } else {
            (1i64 << self.bits()) - 1
        }
    }
}

/// Wrap a value into `ty`, exactly as the language does: "all arithmetic
/// operators wrap on overflow", and a narrowing cast keeps the low bits.
fn narrow(v: i64, ty: Ty) -> i64 {
    let bits = ty.bits();
    let w = v & ((1i64 << bits) - 1);
    if ty.signed() && w > ty.max() {
        w - (1i64 << bits)
    } else {
        w
    }
}

/// The bit pattern that lands in memory, which is what the emulator reports.
fn raw(v: i64, ty: Ty) -> u32 {
    (v & ((1i64 << ty.bits()) - 1)) as u32
}

// ---------------------------------------------------------------------------
// The generated language
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
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

const OPS: [Op; 10] = [
    Op::Add,
    Op::Sub,
    Op::Mul,
    Op::Div,
    Op::Rem,
    Op::And,
    Op::Or,
    Op::Xor,
    Op::Shl,
    Op::Shr,
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Cmp {
    fn sym(self) -> &'static str {
        match self {
            Cmp::Eq => "==",
            Cmp::Ne => "!=",
            Cmp::Lt => "<",
            Cmp::Le => "<=",
            Cmp::Gt => ">",
            Cmp::Ge => ">=",
        }
    }
    fn apply(self, a: i64, b: i64) -> bool {
        match self {
            Cmp::Eq => a == b,
            Cmp::Ne => a != b,
            Cmp::Lt => a < b,
            Cmp::Le => a <= b,
            Cmp::Gt => a > b,
            Cmp::Ge => a >= b,
        }
    }
}

const CMPS: [Cmp; 6] = [Cmp::Eq, Cmp::Ne, Cmp::Lt, Cmp::Le, Cmp::Gt, Cmp::Ge];

/// An expression of the program's one type.
#[derive(Clone)]
enum E {
    Lit(i64),
    Var(usize),
    /// The induction variable of an enclosing `for`, by loop id. It is always
    /// `u8`; at any other program type it appears through a cast.
    Loop(usize),
    Bin(Box<E>, Op, Box<E>),
    /// Out to another type and straight back: truncation and sign-extension,
    /// with no mixed-width arithmetic attached.
    Cast(Ty, Box<E>),
}

/// A condition. Only ever a condition — `bool` as a *value* has its own
/// widening rule, which is a separate question from control flow.
#[derive(Clone)]
enum B {
    Rel(Box<E>, Cmp, Box<E>),
    And(Box<B>, Box<B>),
    Or(Box<B>, Box<B>),
    Not(Box<B>),
}

#[derive(Clone)]
enum S {
    Assign(usize, E),
    If(B, Vec<S>, Option<Vec<S>>),
    /// `for i{id} in 0..{count}`.
    For(usize, u32, Vec<S>),
    /// `while c{id} > 0 { … c{id} = c{id} - 1; }`. The counter is declared at
    /// the top with a positive start value and is written by nothing else, so
    /// the loop terminates by construction.
    While(usize, Vec<S>),
}

const VARS: usize = 4;

// ---------------------------------------------------------------------------
// Generation
// ---------------------------------------------------------------------------

struct Gen<'a> {
    rng: &'a mut Rng,
    ty: Ty,
    /// Loop ids currently in scope, innermost last.
    scope: Vec<usize>,
    /// Start value of each `while` counter, by id.
    counters: Vec<u32>,
    loops: usize,
}

impl Gen<'_> {
    fn lit(&mut self) -> i64 {
        // `-128` as a literal would be unary minus applied to an out-of-range
        // `128`; the value is still reachable by arithmetic.
        let lo = if self.ty.signed() {
            self.ty.min() + 1
        } else {
            self.ty.min()
        };
        lo + self.rng.below((self.ty.max() - lo + 1) as u64) as i64
    }
}

/// `anchored` means the result must mention a variable. A run of pure literals
/// carries no program type: the language types it from the literals' own range,
/// so `0 >= (3 << 7)` is a `u8` comparison even inside an `i8` program, and the
/// shift wraps at eight bits rather than the program's width. Giving every
/// operator at least one anchored operand — the left one, plus a cast's operand
/// — keeps every subexpression at the program's type, which is the type the
/// oracle evaluates at. A bare literal beside an anchored sibling is fine: it
/// adopts the sibling's type.
fn gen_expr(g: &mut Gen, depth: u32, anchored: bool) -> E {
    if depth == 0 || g.rng.below(100) < 30 {
        let choices = if g.scope.is_empty() { 2 } else { 3 };
        let pick = if anchored {
            1 + g.rng.below(choices - 1)
        } else {
            g.rng.below(choices)
        };
        return match pick {
            0 => E::Lit(g.lit()),
            1 => E::Var(g.rng.below(VARS as u64) as usize),
            _ => {
                let k = g.rng.below(g.scope.len() as u64) as usize;
                E::Loop(g.scope[k])
            }
        };
    }

    if g.rng.below(100) < 12 {
        let mut to = TYPES[g.rng.below(TYPES.len() as u64) as usize];
        if to == g.ty {
            to = TYPES[(TYPES.iter().position(|t| *t == g.ty).unwrap() + 1) % TYPES.len()];
        }
        return E::Cast(to, Box::new(gen_expr(g, depth - 1, true)));
    }

    let op = OPS[g.rng.below(OPS.len() as u64) as usize];
    let lhs = gen_expr(g, depth - 1, true);
    let rhs = match op {
        Op::Div | Op::Rem => E::Lit(1 + g.rng.below(g.ty.max() as u64) as i64),
        Op::Shl | Op::Shr => E::Lit(g.rng.below(g.ty.bits() as u64) as i64),
        _ => gen_expr(g, depth - 1, false),
    };
    E::Bin(Box::new(lhs), op, Box::new(rhs))
}

fn gen_bool(g: &mut Gen, depth: u32) -> B {
    if depth > 0 && g.rng.below(100) < 45 {
        return match g.rng.below(3) {
            0 => B::And(
                Box::new(gen_bool(g, depth - 1)),
                Box::new(gen_bool(g, depth - 1)),
            ),
            1 => B::Or(
                Box::new(gen_bool(g, depth - 1)),
                Box::new(gen_bool(g, depth - 1)),
            ),
            _ => B::Not(Box::new(gen_bool(g, depth - 1))),
        };
    }
    let l = gen_expr(g, 2, true);
    let cmp = CMPS[g.rng.below(CMPS.len() as u64) as usize];
    let r = gen_expr(g, 2, false);
    B::Rel(Box::new(l), cmp, Box::new(r))
}

fn gen_block(g: &mut Gen, depth: u32) -> Vec<S> {
    let n = 1 + g.rng.below(if depth == 0 { 2 } else { 3 }) as usize;
    (0..n).map(|_| gen_stmt(g, depth)).collect()
}

fn gen_stmt(g: &mut Gen, depth: u32) -> S {
    let pick = g.rng.below(100);
    if depth == 0 || pick < 50 {
        let v = g.rng.below(VARS as u64) as usize;
        // The assignment target supplies the type, so the right-hand side does
        // not need its own anchor.
        return S::Assign(v, gen_expr(g, 3, false));
    }
    match pick {
        50..=71 => {
            let cond = gen_bool(g, 2);
            let then = gen_block(g, depth - 1);
            let otherwise = if g.rng.below(2) == 0 {
                Some(gen_block(g, depth - 1))
            } else {
                None
            };
            S::If(cond, then, otherwise)
        }
        72..=87 => {
            let id = g.loops;
            g.loops += 1;
            let count = 1 + g.rng.below(4) as u32;
            g.scope.push(id);
            let body = gen_block(g, depth - 1);
            g.scope.pop();
            S::For(id, count, body)
        }
        _ => {
            let id = g.counters.len();
            let start = 1 + g.rng.below(4) as u32;
            g.counters.push(start);
            let body = gen_block(g, depth - 1);
            S::While(id, body)
        }
    }
}

#[derive(Clone)]
struct Prog {
    ty: Ty,
    init: [i64; VARS],
    counters: Vec<u32>,
    stmts: Vec<S>,
    loops: usize,
}

fn gen_program(seed: u64) -> Prog {
    let mut rng = Rng::new(seed);
    let ty = TYPES[rng.below(TYPES.len() as u64) as usize];
    let mut init = [0i64; VARS];
    let mut g = Gen {
        rng: &mut rng,
        ty,
        scope: Vec::new(),
        counters: Vec::new(),
        loops: 0,
    };
    for v in init.iter_mut() {
        *v = g.lit();
    }
    let n = 2 + g.rng.below(3) as usize;
    let stmts = (0..n).map(|_| gen_stmt(&mut g, 2)).collect();
    let (counters, loops) = (g.counters, g.loops);
    Prog {
        ty,
        init,
        counters,
        stmts,
        loops,
    }
}

// ---------------------------------------------------------------------------
// The oracle: run the same tree in Rust
// ---------------------------------------------------------------------------

struct St {
    vars: [i64; VARS],
    counters: Vec<i64>,
    loops: Vec<i64>,
}

fn eval(e: &E, st: &St, ty: Ty) -> i64 {
    match e {
        E::Lit(v) => narrow(*v, ty),
        E::Var(i) => st.vars[*i],
        E::Loop(id) => narrow(st.loops[*id], ty),
        E::Cast(to, inner) => narrow(narrow(eval(inner, st, ty), *to), ty),
        E::Bin(l, op, r) => {
            let a = eval(l, st, ty);
            let b = eval(r, st, ty);
            let v = match op {
                Op::Add => a.wrapping_add(b),
                Op::Sub => a.wrapping_sub(b),
                Op::Mul => a.wrapping_mul(b),
                // Generated divisors are positive literals; the guard is
                // belt-and-braces so a generator change cannot panic here.
                Op::Div => a / if b == 0 { 1 } else { b },
                Op::Rem => a % if b == 0 { 1 } else { b },
                // Bitwise operators act on the value's bit pattern, so a
                // negative operand has to be seen unsigned first.
                Op::And => (raw(a, ty) & raw(b, ty)) as i64,
                Op::Or => (raw(a, ty) | raw(b, ty)) as i64,
                Op::Xor => (raw(a, ty) ^ raw(b, ty)) as i64,
                Op::Shl => a << b.clamp(0, ty.bits() as i64 - 1),
                // `>>` on a signed type is arithmetic, which is what Rust's
                // `>>` on `i64` already does for our sign-correct values.
                Op::Shr => a >> b.clamp(0, ty.bits() as i64 - 1),
            };
            narrow(v, ty)
        }
    }
}

fn eval_bool(b: &B, st: &St, ty: Ty) -> bool {
    match b {
        B::Rel(l, c, r) => c.apply(eval(l, st, ty), eval(r, st, ty)),
        B::And(l, r) => eval_bool(l, st, ty) && eval_bool(r, st, ty),
        B::Or(l, r) => eval_bool(l, st, ty) || eval_bool(r, st, ty),
        B::Not(inner) => !eval_bool(inner, st, ty),
    }
}

fn exec(stmts: &[S], st: &mut St, ty: Ty) {
    for s in stmts {
        match s {
            S::Assign(v, e) => st.vars[*v] = eval(e, st, ty),
            S::If(c, then, otherwise) => {
                if eval_bool(c, st, ty) {
                    exec(then, st, ty);
                } else if let Some(e) = otherwise {
                    exec(e, st, ty);
                }
            }
            S::For(id, count, body) => {
                for i in 0..*count as i64 {
                    st.loops[*id] = i;
                    exec(body, st, ty);
                }
            }
            S::While(id, body) => {
                while st.counters[*id] > 0 {
                    exec(body, st, ty);
                    st.counters[*id] -= 1;
                }
            }
        }
    }
}

/// The final bit pattern of every program variable.
fn expected(p: &Prog) -> Vec<u32> {
    let mut st = St {
        vars: p.init,
        counters: p.counters.iter().map(|c| *c as i64).collect(),
        loops: vec![0; p.loops],
    };
    exec(&p.stmts, &mut st, p.ty);
    st.vars.iter().map(|v| raw(*v, p.ty)).collect()
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

fn render(e: &E, ty: Ty) -> String {
    match e {
        E::Lit(v) if *v < 0 => format!("({v})"),
        E::Lit(v) => format!("{v}"),
        E::Var(i) => format!("v{i}"),
        E::Loop(id) if ty == Ty::U8 => format!("i{id}"),
        E::Loop(id) => format!("(i{id} as {})", ty.name()),
        E::Bin(l, op, r) => format!("({} {} {})", render(l, ty), op.sym(), render(r, ty)),
        E::Cast(to, inner) => format!(
            "(({} as {}) as {})",
            render(inner, ty),
            to.name(),
            ty.name()
        ),
    }
}

fn render_bool(b: &B, ty: Ty) -> String {
    match b {
        B::Rel(l, c, r) => format!("({} {} {})", render(l, ty), c.sym(), render(r, ty)),
        B::And(l, r) => format!("({} && {})", render_bool(l, ty), render_bool(r, ty)),
        B::Or(l, r) => format!("({} || {})", render_bool(l, ty), render_bool(r, ty)),
        B::Not(inner) => format!("(!{})", render_bool(inner, ty)),
    }
}

fn render_stmts(stmts: &[S], ty: Ty, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let mut out = String::new();
    for s in stmts {
        match s {
            S::Assign(v, e) => out.push_str(&format!("{pad}v{v} = {};\n", render(e, ty))),
            S::If(c, then, otherwise) => {
                out.push_str(&format!("{pad}if {} {{\n", render_bool(c, ty)));
                out.push_str(&render_stmts(then, ty, indent + 4));
                match otherwise {
                    Some(e) => {
                        out.push_str(&format!("{pad}}} else {{\n"));
                        out.push_str(&render_stmts(e, ty, indent + 4));
                        out.push_str(&format!("{pad}}}\n"));
                    }
                    None => out.push_str(&format!("{pad}}}\n")),
                }
            }
            S::For(id, count, body) => {
                out.push_str(&format!("{pad}for i{id} in 0..{count} {{\n"));
                out.push_str(&render_stmts(body, ty, indent + 4));
                out.push_str(&format!("{pad}}}\n"));
            }
            S::While(id, body) => {
                out.push_str(&format!("{pad}while c{id} > 0 {{\n"));
                out.push_str(&render_stmts(body, ty, indent + 4));
                out.push_str(&format!("{pad}    c{id} = c{id} - 1;\n"));
                out.push_str(&format!("{pad}}}\n"));
            }
        }
    }
    out
}

/// The surface forms one program is written in. All must agree; each routes
/// through a different part of codegen.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Form {
    /// Statements directly in `main`.
    Inline,
    /// The whole body in a called function.
    ViaFunction,
    /// The statements inside the taken arm of a `match`.
    ViaMatch,
    /// The statements inside a loop that runs exactly once.
    ViaLoop,
}

const FORMS: [Form; 4] = [
    Form::Inline,
    Form::ViaFunction,
    Form::ViaMatch,
    Form::ViaLoop,
];

/// One output byte per 8-bit variable, two per 16-bit one.
fn out_bytes(ty: Ty) -> usize {
    VARS * if ty.wide() { 2 } else { 1 }
}

fn render_program(p: &Prog, form: Form) -> String {
    let ty = p.ty;
    let tn = ty.name();

    let head: String = (0..out_bytes(ty))
        .map(|i| format!("const OUT{i}: addr = 0x{:04X};\n", 0x0900 + i))
        .collect();

    let mut decls = String::new();
    for (i, v) in p.init.iter().enumerate() {
        let lit = if *v < 0 {
            format!("({v})")
        } else {
            format!("{v}")
        };
        decls.push_str(&format!("    let v{i}: {tn} = {lit};\n"));
    }
    for (i, c) in p.counters.iter().enumerate() {
        decls.push_str(&format!("    let c{i}: u8 = {c};\n"));
    }

    let mut stores = String::new();
    for i in 0..VARS {
        if ty.wide() {
            let src = if ty.signed() {
                format!("v{i} as u16")
            } else {
                format!("v{i}")
            };
            stores.push_str(&format!("    let o{i}: u16 = {src};\n"));
            stores.push_str(&format!("    OUT{} = o{i}.low;\n", i * 2));
            stores.push_str(&format!("    OUT{} = o{i}.high;\n", i * 2 + 1));
        } else {
            let src = if ty.signed() {
                format!("v{i} as u8")
            } else {
                format!("v{i}")
            };
            stores.push_str(&format!("    OUT{i} = {src};\n"));
        }
    }

    // Re-indent a block of already-formatted statements by one more level.
    let bump = |s: &str| -> String { s.lines().map(|l| format!("    {l}\n")).collect::<String>() };

    let body = render_stmts(&p.stmts, ty, 4);

    match form {
        Form::Inline => {
            format!("{head}#[reset]\nfn main() {{\n{decls}{body}{stores}    loop {{}}\n}}\n")
        }
        Form::ViaFunction => format!(
            "{head}fn body() {{\n{decls}{body}{stores}}}\n\
             #[reset]\nfn main() {{\n    body();\n    loop {{}}\n}}\n"
        ),
        Form::ViaMatch => format!(
            "{head}#[reset]\nfn main() {{\n{decls}    let sel: u8 = 0;\n\
                 match sel {{\n        0 => {{\n{}{}        }}\n        _ => {{}}\n    }}\n\
             \x20   loop {{}}\n}}\n",
            bump(&bump(&body)),
            bump(&bump(&stores)),
        ),
        Form::ViaLoop => format!(
            "{head}#[reset]\nfn main() {{\n{decls}    for w0 in 0..1 {{\n{}{}    }}\n\
             \x20   loop {{}}\n}}\n",
            bump(&body),
            bump(&stores),
        ),
    }
}

// ---------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------

/// Compile and run one program, reading back every variable, or reporting the
/// reason it did not get there.
fn observe(src: &str, ty: Ty) -> Result<Vec<u32>, String> {
    let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut e = run(src);
        (0..VARS)
            .map(|i| {
                if ty.wide() {
                    e.mem16(0x0900 + (i * 2) as u16) as u32
                } else {
                    e.mem(0x0900 + i as u16) as u32
                }
            })
            .collect::<Vec<u32>>()
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
        .unwrap_or(120)
}

/// Why a program failed. Shrinking has to preserve the *kind*: a reduction
/// that turns a wrong answer into a rejected program has found a different bug
/// and would report the wrong one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Ran, and produced values the oracle disagrees with.
    WrongAnswer,
    /// Never ran — rejected by the compiler, the assembler, or the emulator.
    Rejected,
}

/// How this program disagrees with the oracle in one surface form, if it does.
fn disagrees_as(p: &Prog, form: Form) -> Option<(Kind, String)> {
    let want = expected(p);
    match observe(&render_program(p, form), p.ty) {
        Ok(got) if got == want => None,
        Ok(got) => Some((
            Kind::WrongAnswer,
            format!("gave {got:?}, expected {want:?}"),
        )),
        Err(why) => Some((Kind::Rejected, format!("failed to run: {why}"))),
    }
}

/// The first surface form that disagrees, if any.
fn disagrees(p: &Prog) -> Option<(Form, Kind, String)> {
    FORMS
        .into_iter()
        .find_map(|f| disagrees_as(p, f).map(|(k, why)| (f, k, why)))
}

// ---------------------------------------------------------------------------
// Shrinking
// ---------------------------------------------------------------------------
//
// A generated program is 30-odd lines of dense arithmetic, and the part of it
// that matters is usually one operator. Reducing it before reporting is the
// difference between a finding a person can act on and one they have to
// re-derive by hand. Every candidate is re-run, so whatever survives still
// fails for the same reason it did before.

/// Apply the `target`-th available simplification, counting them in `seen`.
/// With a `target` no index can reach, this only counts.
fn mutate(p: &mut Prog, target: usize, seen: &mut usize) -> bool {
    for i in 0..VARS {
        if p.init[i] != 0 {
            if *seen == target {
                p.init[i] = 0;
                return true;
            }
            *seen += 1;
        }
    }
    for i in 0..p.counters.len() {
        if p.counters[i] > 1 {
            if *seen == target {
                p.counters[i] = 1;
                return true;
            }
            *seen += 1;
        }
    }
    mutate_block(&mut p.stmts, target, seen)
}

fn mutate_block(block: &mut Vec<S>, target: usize, seen: &mut usize) -> bool {
    // Never empty a block: an enclosing statement gets dropped whole instead.
    if block.len() > 1 {
        for i in 0..block.len() {
            if *seen == target {
                block.remove(i);
                return true;
            }
            *seen += 1;
        }
    }
    for s in block.iter_mut() {
        if mutate_stmt(s, target, seen) {
            return true;
        }
    }
    false
}

fn mutate_stmt(s: &mut S, target: usize, seen: &mut usize) -> bool {
    match s {
        S::Assign(_, e) => mutate_expr(e, target, seen),
        S::If(cond, then, otherwise) => {
            if mutate_bool(cond, target, seen) {
                return true;
            }
            if otherwise.is_some() {
                if *seen == target {
                    *otherwise = None;
                    return true;
                }
                *seen += 1;
            }
            if mutate_block(then, target, seen) {
                return true;
            }
            match otherwise {
                Some(b) => mutate_block(b, target, seen),
                None => false,
            }
        }
        S::For(_, count, body) => {
            if *count > 1 {
                if *seen == target {
                    *count = 1;
                    return true;
                }
                *seen += 1;
            }
            mutate_block(body, target, seen)
        }
        S::While(_, body) => mutate_block(body, target, seen),
    }
}

fn mutate_expr(e: &mut E, target: usize, seen: &mut usize) -> bool {
    match e {
        E::Lit(v) => {
            if *v != 0 {
                if *seen == target {
                    *v = 0;
                    return true;
                }
                *seen += 1;
            }
            false
        }
        E::Cast(_, inner) => {
            if *seen == target {
                let lifted = (**inner).clone();
                *e = lifted;
                return true;
            }
            *seen += 1;
            mutate_expr(inner, target, seen)
        }
        E::Bin(l, op, r) => {
            if *seen == target {
                let lifted = (**l).clone();
                *e = lifted;
                return true;
            }
            *seen += 1;
            if *seen == target {
                let lifted = (**r).clone();
                *e = lifted;
                return true;
            }
            *seen += 1;
            let op = *op;
            if mutate_expr(l, target, seen) {
                return true;
            }
            // The divisor is a nonzero literal by construction, and zeroing it
            // would leave a program the oracle is not entitled to an answer for.
            if matches!(op, Op::Div | Op::Rem) {
                return false;
            }
            mutate_expr(r, target, seen)
        }
        _ => false,
    }
}

fn mutate_bool(b: &mut B, target: usize, seen: &mut usize) -> bool {
    match b {
        B::Rel(l, _, r) => mutate_expr(l, target, seen) || mutate_expr(r, target, seen),
        B::And(l, r) | B::Or(l, r) => {
            if *seen == target {
                let lifted = (**l).clone();
                *b = lifted;
                return true;
            }
            *seen += 1;
            if *seen == target {
                let lifted = (**r).clone();
                *b = lifted;
                return true;
            }
            *seen += 1;
            mutate_bool(l, target, seen) || mutate_bool(r, target, seen)
        }
        B::Not(inner) => {
            if *seen == target {
                let lifted = (**inner).clone();
                *b = lifted;
                return true;
            }
            *seen += 1;
            mutate_bool(inner, target, seen)
        }
    }
}

/// The smallest program reachable by repeated one-step simplification that
/// still fails in `form`. Bounded, because each candidate costs a compile and a
/// run and a pathological case should not hang the suite.
fn shrink(p: &Prog, form: Form, kind: Kind) -> Prog {
    const BUDGET: usize = 1500;
    let mut best = p.clone();
    let mut spent = 0usize;
    loop {
        let mut available = 0usize;
        mutate(&mut best.clone(), usize::MAX, &mut available);

        let mut improved = false;
        for k in 0..available {
            if spent >= BUDGET {
                return best;
            }
            let mut cand = best.clone();
            let mut seen = 0usize;
            if !mutate(&mut cand, k, &mut seen) {
                continue;
            }
            spent += 1;
            if disagrees_as(&cand, form).is_some_and(|(k, _)| k == kind) {
                best = cand;
                improved = true;
                break;
            }
        }
        if !improved {
            return best;
        }
    }
}

/// One generated case, checked against the oracle in every surface form.
fn check_seed(seed: u64) -> Result<(), String> {
    let p = gen_program(seed);
    let Some((form, kind, _)) = disagrees(&p) else {
        return Ok(());
    };

    let small = shrink(&p, form, kind);
    let why = disagrees_as(&small, form)
        .map(|(_, why)| why)
        .unwrap_or_else(|| "no longer reproduces".into());
    Err(format!(
        "seed {seed}: {form:?} form {why}\ntype: {}\n--- reduced source ---\n{}",
        small.ty.name(),
        render_program(&small, form)
    ))
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
    for seed in [0u64, 1, 42, 9999] {
        let a = gen_program(seed);
        let b = gen_program(seed);
        assert_eq!(
            render_program(&a, Form::Inline),
            render_program(&b, Form::Inline),
            "seed {seed} produced two different programs"
        );
        assert_eq!(expected(&a), expected(&b), "seed {seed} evaluated twice");
    }
}

/// The oracle has to be exercised by the shapes it claims to cover, or a
/// generator change could quietly stop producing them.
#[test]
fn the_generator_covers_what_it_claims() {
    #[derive(Default)]
    struct Seen {
        ops: std::collections::HashSet<&'static str>,
        cmps: std::collections::HashSet<&'static str>,
        stmts: std::collections::HashSet<&'static str>,
        bools: std::collections::HashSet<&'static str>,
        types: std::collections::HashSet<&'static str>,
        casts: usize,
        loop_vars: usize,
    }

    fn walk_e(e: &E, s: &mut Seen) {
        match e {
            E::Bin(l, op, r) => {
                s.ops.insert(op.sym());
                walk_e(l, s);
                walk_e(r, s);
            }
            E::Cast(_, inner) => {
                s.casts += 1;
                walk_e(inner, s);
            }
            E::Loop(_) => s.loop_vars += 1,
            _ => {}
        }
    }
    fn walk_b(b: &B, s: &mut Seen) {
        match b {
            B::Rel(l, c, r) => {
                s.cmps.insert(c.sym());
                walk_e(l, s);
                walk_e(r, s);
            }
            B::And(l, r) => {
                s.bools.insert("&&");
                walk_b(l, s);
                walk_b(r, s);
            }
            B::Or(l, r) => {
                s.bools.insert("||");
                walk_b(l, s);
                walk_b(r, s);
            }
            B::Not(i) => {
                s.bools.insert("!");
                walk_b(i, s);
            }
        }
    }
    fn walk_s(stmts: &[S], s: &mut Seen) {
        for st in stmts {
            match st {
                S::Assign(_, e) => {
                    s.stmts.insert("assign");
                    walk_e(e, s);
                }
                S::If(c, t, e) => {
                    s.stmts.insert(if e.is_some() { "if-else" } else { "if" });
                    walk_b(c, s);
                    walk_s(t, s);
                    if let Some(e) = e {
                        walk_s(e, s);
                    }
                }
                S::For(_, _, b) => {
                    s.stmts.insert("for");
                    walk_s(b, s);
                }
                S::While(_, b) => {
                    s.stmts.insert("while");
                    walk_s(b, s);
                }
            }
        }
    }

    let mut seen = Seen::default();
    for seed in 0..400u64 {
        let p = gen_program(seed);
        seen.types.insert(p.ty.name());
        walk_s(&p.stmts, &mut seen);
    }

    for op in OPS {
        assert!(seen.ops.contains(op.sym()), "never emitted `{}`", op.sym());
    }
    for c in CMPS {
        assert!(seen.cmps.contains(c.sym()), "never emitted `{}`", c.sym());
    }
    for k in ["assign", "if", "if-else", "for", "while"] {
        assert!(seen.stmts.contains(k), "never emitted a `{k}` statement");
    }
    for k in ["&&", "||", "!"] {
        assert!(seen.bools.contains(k), "never emitted `{k}`");
    }
    for t in TYPES {
        assert!(seen.types.contains(t.name()), "never used `{}`", t.name());
    }
    assert!(seen.casts > 0, "never emitted a cast");
    assert!(seen.loop_vars > 0, "never read a loop variable");
}

/// The oracle's own arithmetic, pinned against hand-checked values. If this
/// drifts, every "miscompile" it reports is suspect.
#[test]
fn the_oracle_matches_the_documented_semantics() {
    let cases: &[(Ty, i64, Op, i64, i64)] = &[
        // Wrapping, both directions.
        (Ty::U8, 200, Op::Add, 100, 44),
        (Ty::I8, -100, Op::Sub, 100, 56),
        (Ty::U16, 65535, Op::Add, 2, 1),
        // Signed division truncates toward zero; `%` follows the dividend.
        (Ty::I8, -7, Op::Div, 2, -3),
        (Ty::I8, -7, Op::Rem, 2, -1),
        (Ty::I16, -3, Op::Div, 4, 0),
        // `>>` is arithmetic on a signed type, logical on an unsigned one.
        (Ty::I8, -8, Op::Shr, 1, -4),
        (Ty::U8, 255, Op::Shr, 1, 127),
        // `<<` wraps like any other arithmetic.
        (Ty::I8, 100, Op::Shl, 1, -56),
        // Bitwise operators see the bit pattern, not the signed value.
        (Ty::I8, -1, Op::And, 15, 15),
        (Ty::I8, -128, Op::Xor, -128, 0),
    ];
    let st = St {
        vars: [0; VARS],
        counters: Vec::new(),
        loops: Vec::new(),
    };
    for (ty, a, op, b, want) in cases {
        let e = E::Bin(Box::new(E::Lit(*a)), *op, Box::new(E::Lit(*b)));
        assert_eq!(
            eval(&e, &st, *ty),
            *want,
            "{} {} {} at {}",
            a,
            op.sym(),
            b,
            ty.name()
        );
    }
}

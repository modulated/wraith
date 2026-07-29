//! Escape analysis for pointers.
//!
//! Locals live in zero-page frames laid out by call-graph colouring
//! (`super::frames`): an SCC's base sits above the end of every predecessor's,
//! so **a callee's frame never overlaps a live caller's**. That invariant is why
//! passing `&local` down into a callee is safe, and it is the only thing making
//! it safe — the language has done this implicitly for struct arguments all
//! along. What it does not cover is a pointer travelling the other way. Once a
//! function returns, an unrelated function may occupy the same bytes.
//!
//! So each pointer-valued expression gets a provenance, and four rules are
//! checked against it. This is deliberately not a borrow checker: it has no
//! lifetimes, no regions and no annotations, and it accepts plenty of programs a
//! borrow checker would reject. It exists to stop the specific shapes that
//! silently miscompile.
//!
//! ## Why `Unknown` can be permissive
//!
//! A pointer *parameter* has unknown provenance — inside the callee there is no
//! way to tell whether the caller passed a static or a local. Treating that as
//! `Local` would make a registration function (`fn keep(d: &Device) { TABLE = d; }`)
//! impossible to write; treating it as `Global` would be a lie.
//!
//! It is safe to treat as permissive *because* of an induction: rules 1 and 2
//! guarantee nothing `Local` is ever stored in a global or written through a
//! pointer, so any pointer read back out of one points at stable storage. The
//! remaining hole — a callee storing a caller's `&local` — is closed from the
//! other side by rule 5, which checks the call site against a per-parameter
//! escape summary. Removing either half makes the analysis decorative, so do not
//! "simplify" one away without the other.

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use crate::ast::{Expr, Item, SourceFile, Span, Spanned, Stmt, UnaryOp};
use crate::sema::SemaError;
use crate::sema::table::SymbolKind;
use crate::sema::types::Type;

use super::SemanticAnalyzer;

/// Where a pointer came from, ordered `Local ⊏ Unknown ⊏ Global`.
///
/// Joining takes the minimum, so a variable that is ever assigned a `Local`
/// pointer is `Local` for its whole lifetime. That is flow-insensitive on
/// purpose: it needs one pass, and it makes `let p = &x; return p;` fall out of
/// the return rule rather than needing its own case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Provenance {
    /// Points into the current function's frame.
    Local,
    /// A parameter, a deref result, or something loaded back out of memory.
    Unknown,
    /// A `static`, or an integer cast to a pointer.
    Global,
}

/// Facts gathered for one function.
#[derive(Default)]
struct FnFacts {
    /// Provenance of each pointer-valued local, keyed by name. Names are unique
    /// enough here: a shadowed name in a sibling scope only ever makes this
    /// *more* conservative, because provenances are met rather than replaced.
    locals: HashMap<String, Provenance>,
}

impl SemanticAnalyzer {
    /// Check every pointer rule over the whole program.
    ///
    /// Runs after `finalize_frames`, which is what supplies the recursion SCCs,
    /// and over the AST rather than during type checking, because two of the
    /// rules need whole-program information that does not exist until every
    /// body has been seen.
    pub(super) fn check_pointer_escapes(
        &mut self,
        root: &SourceFile,
        recursive_edges: &HashSet<(String, String)>,
    ) -> Result<(), SemaError> {
        let functions = self.collect_functions(root);
        if functions.is_empty() {
            return Ok(());
        }

        // Functions that take part in a recursion cycle, in either direction.
        let mut recursive: HashSet<String> = HashSet::default();
        for (a, b) in recursive_edges {
            recursive.insert(a.clone());
            recursive.insert(b.clone());
        }

        // Pass 1: provenance of every pointer-valued local.
        let mut facts: HashMap<String, FnFacts> = HashMap::default();
        for (name, func) in &functions {
            let mut f = FnFacts::default();
            self.collect_provenance(&func.body, &mut f);
            facts.insert(name.clone(), f);
        }

        // Pass 2: which parameters escape, to a fixpoint over the call graph.
        let escapes = self.compute_escape_summary(&functions, &facts);

        // Pass 3: the rules.
        for (name, func) in &functions {
            let f = &facts[name];
            let params: Vec<String> = func.params.iter().map(|p| p.name.node.clone()).collect();
            self.check_body(
                &func.body, name, f, &params, &recursive, &escapes, &functions,
            )?;
        }
        Ok(())
    }

    /// Every function in the program, root module and imports alike.
    fn collect_functions(&self, root: &SourceFile) -> Vec<(String, crate::ast::Function)> {
        let mut out = Vec::new();
        let mut seen: HashSet<String> = HashSet::default();
        for item in self.imported_items.iter().chain(root.items.iter()) {
            if let Item::Function(f) = &item.node
                && seen.insert(f.name.node.clone())
            {
                out.push((f.name.node.clone(), (**f).clone()));
            }
        }
        out
    }

    /// Meet every pointer value ever bound to each local.
    fn collect_provenance(&self, body: &Spanned<Stmt>, facts: &mut FnFacts) {
        walk_stmts(body, &mut |stmt| {
            let (name, init) = match &stmt.node {
                Stmt::VarDecl { name, init, .. } => (name.node.clone(), init),
                Stmt::Assign { target, value } => match &target.node {
                    Expr::Variable(n) => (n.clone(), value),
                    _ => return,
                },
                _ => return,
            };
            if !self.is_pointer_expr(init) {
                return;
            }
            let p = self.provenance_of(init, facts);
            facts
                .locals
                .entry(name)
                .and_modify(|e| *e = (*e).min(p))
                .or_insert(p);
        });
    }

    /// Is this expression pointer-typed?
    fn is_pointer_expr(&self, e: &Spanned<Expr>) -> bool {
        matches!(self.resolved_types.get(&e.span), Some(Type::Pointer(_)))
            || matches!(
                &e.node,
                Expr::Unary {
                    op: UnaryOp::AddrOf,
                    ..
                }
            )
    }

    fn provenance_of(&self, e: &Spanned<Expr>, facts: &FnFacts) -> Provenance {
        match &e.node {
            Expr::Paren(inner) => self.provenance_of(inner, facts),

            // `&x` — global storage keeps its address for the life of the
            // program; a frame slot does not.
            Expr::Unary {
                op: UnaryOp::AddrOf,
                operand,
            } => match self.addr_of_target(operand) {
                Some(true) => Provenance::Global,
                Some(false) => Provenance::Local,
                None => Provenance::Unknown,
            },

            // A pointer read back out of a variable carries that variable's
            // provenance; a parameter has none recorded and is Unknown.
            Expr::Variable(name) => facts
                .locals
                .get(name)
                .copied()
                .unwrap_or(Provenance::Unknown),

            // An integer cast to a pointer is the documented escape hatch, and
            // the programmer is asserting it is stable.
            Expr::Cast { .. } => Provenance::Global,

            _ => Provenance::Unknown,
        }
    }

    /// For `&operand`, is the target global storage? `None` when it cannot be
    /// determined, which is treated as `Unknown`.
    fn addr_of_target(&self, operand: &Spanned<Expr>) -> Option<bool> {
        let inner = match &operand.node {
            Expr::Paren(i) => &**i,
            Expr::Index { object, .. } => object,
            Expr::Field { object, .. } => object,
            // A `.len`/`.low`/`.high` re-resolved as a struct field: the
            // address is the object's storage, same as a plain field.
            Expr::SliceLen(i) | Expr::U16Low(i) | Expr::U16High(i)
                if self.accessor_fields.contains(&operand.span) =>
            {
                &**i
            }
            _ => operand,
        };
        let sym = self.resolved_symbols.get(&inner.span)?;
        if matches!(sym.kind, SymbolKind::Function | SymbolKind::Address) {
            return Some(true);
        }
        // A global has no containing function; a local or parameter does.
        Some(sym.containing_function.is_none())
    }

    /// Which parameters of each function let a pointer escape.
    ///
    /// Seeded from stores of a parameter into a global or through a pointer,
    /// then propagated along the call graph: if `f` hands its parameter *i* to
    /// `g`'s parameter *j* and `g`'s *j* escapes, so does `f`'s *i*. Monotone
    /// and bounded by parameters × functions, so a handful of rounds.
    fn compute_escape_summary(
        &self,
        functions: &[(String, crate::ast::Function)],
        facts: &HashMap<String, FnFacts>,
    ) -> HashMap<String, HashSet<usize>> {
        let mut escapes: HashMap<String, HashSet<usize>> = HashMap::default();
        // Forwarding edges: (caller, caller_param_index) -> (callee, index).
        let mut forwards: Vec<(String, usize, String, usize)> = Vec::new();

        for (name, func) in functions {
            let params: Vec<String> = func.params.iter().map(|p| p.name.node.clone()).collect();
            let mut direct: HashSet<usize> = HashSet::default();

            walk_stmts(&func.body, &mut |stmt| {
                // A store into anything that outlives the frame. Returning a
                // parameter is *not* an escape: the caller already had it.
                if let Stmt::Assign { target, value } = &stmt.node
                    && stores_beyond_the_frame(
                        &self.resolved_symbols,
                        &self.accessor_fields,
                        target,
                    )
                    && let Some(i) = param_index(value, &params)
                {
                    direct.insert(i);
                }
                // Forwarding to another function.
                walk_calls(stmt, &mut |callee, args| {
                    for (j, arg) in args.iter().enumerate() {
                        if let Some(i) = param_index(arg, &params) {
                            forwards.push((name.clone(), i, callee.to_string(), j));
                        }
                    }
                });
            });

            if !direct.is_empty() {
                escapes.entry(name.clone()).or_default().extend(direct);
            }
            let _ = facts;
        }

        // Fixpoint.
        loop {
            let mut changed = false;
            for (caller, ci, callee, cj) in &forwards {
                if escapes.get(callee).is_some_and(|s| s.contains(cj))
                    && escapes.entry(caller.clone()).or_default().insert(*ci)
                {
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        escapes
    }

    #[allow(clippy::too_many_arguments)]
    fn check_body(
        &self,
        body: &Spanned<Stmt>,
        fn_name: &str,
        facts: &FnFacts,
        params: &[String],
        recursive: &HashSet<String>,
        escapes: &HashMap<String, HashSet<usize>>,
        functions: &[(String, crate::ast::Function)],
    ) -> Result<(), SemaError> {
        let mut error: Option<SemaError> = None;
        let fail = |e: SemaError, slot: &mut Option<SemaError>| {
            if slot.is_none() {
                *slot = Some(e);
            }
        };

        walk_stmts(body, &mut |stmt| {
            if error.is_some() {
                return;
            }
            match &stmt.node {
                // Rule 1: a pointer into this frame must not be returned.
                Stmt::Return(Some(e)) => {
                    if self.is_pointer_expr(e) && self.provenance_of(e, facts) == Provenance::Local
                    {
                        fail(
                            SemaError::EscapingPointer {
                                reason: format!(
                                    "'{}' returns a pointer to one of its own locals; the frame \
                                     it names is reused by unrelated functions as soon as '{}' \
                                     returns",
                                    fn_name, fn_name
                                ),
                                span: e.span,
                            },
                            &mut error,
                        );
                    }
                }
                // Rule 2: nor stored anywhere that outlives the frame.
                Stmt::Assign { target, value } => {
                    if self.is_pointer_expr(value)
                        && self.provenance_of(value, facts) == Provenance::Local
                        && stores_beyond_the_frame(
                            &self.resolved_symbols,
                            &self.accessor_fields,
                            target,
                        )
                    {
                        fail(
                            SemaError::EscapingPointer {
                                reason: "storing a pointer to a local in a global outlives the \
                                         frame it names"
                                    .to_string(),
                                span: value.span,
                            },
                            &mut error,
                        );
                    }
                }
                _ => {}
            }

            // Rule 3: `&local` inside a recursion cycle.
            if recursive.contains(fn_name) {
                walk_exprs_in_stmt(stmt, &mut |e| {
                    if error.is_some() {
                        return;
                    }
                    if let Expr::Unary {
                        op: UnaryOp::AddrOf,
                        operand,
                    } = &e.node
                        && self.addr_of_target(operand) == Some(false)
                    {
                        fail(
                            SemaError::EscapingPointer {
                                reason: format!(
                                    "cannot take the address of a local in '{}', which is \
                                     recursive: a recursive call saves and restores the frame \
                                     through the software stack, so the same bytes serve every \
                                     depth and a write through the pointer is discarded on \
                                     return. Use a `static` instead",
                                    fn_name
                                ),
                                span: e.span,
                            },
                            &mut error,
                        );
                    }
                });
            }

            // Rules 4 and 5: arguments at a call site.
            walk_calls(stmt, &mut |callee, args| {
                if error.is_some() {
                    return;
                }
                for (i, arg) in args.iter().enumerate() {
                    if !self.is_pointer_expr(arg) {
                        continue;
                    }
                    let prov = self.provenance_of(arg, facts);
                    let is_known_fn = functions.iter().any(|(n, _)| n == callee);
                    if !is_known_fn {
                        // An indirect call: no call edge was recorded for it, so
                        // frame colouring never saw it and cannot protect the
                        // callee's frame from overlapping this one.
                        if prov < Provenance::Global {
                            fail(
                                SemaError::EscapingPointer {
                                    reason: "only a pointer to global storage may cross an \
                                             indirect call; frame colouring does not see calls \
                                             made through a function pointer"
                                        .to_string(),
                                    span: arg.span,
                                },
                                &mut error,
                            );
                        }
                        continue;
                    }
                    if prov == Provenance::Local
                        && escapes.get(callee).is_some_and(|s| s.contains(&i))
                    {
                        fail(
                            SemaError::EscapingPointer {
                                reason: format!(
                                    "'{}' stores this parameter somewhere that outlives the \
                                     call, so it must not be given a pointer to a local",
                                    callee
                                ),
                                span: arg.span,
                            },
                            &mut error,
                        );
                    }
                }
            });
        });

        let _ = params;
        match error {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

/// Does assigning to `target` write somewhere that outlives the current frame?
///
/// A global, or the target of a pointer store — in the latter case the pointer
/// may name anything at all, so it has to be assumed to outlive us.
fn stores_beyond_the_frame(
    resolved: &HashMap<Span, crate::sema::table::SymbolInfo>,
    accessor_fields: &HashSet<Span>,
    target: &Spanned<Expr>,
) -> bool {
    match &target.node {
        Expr::Variable(_) => resolved
            .get(&target.span)
            .is_some_and(|s| s.containing_function.is_none()),
        Expr::Unary {
            op: UnaryOp::Deref, ..
        } => true,
        Expr::Field { object, .. } | Expr::Index { object, .. } => {
            stores_beyond_the_frame(resolved, accessor_fields, object)
        }
        // A `.len`/`.low`/`.high` that sema re-resolved as a struct field
        // peels exactly like a plain field access.
        Expr::SliceLen(inner) | Expr::U16Low(inner) | Expr::U16High(inner)
            if accessor_fields.contains(&target.span) =>
        {
            stores_beyond_the_frame(resolved, accessor_fields, inner)
        }
        _ => false,
    }
}

/// If `e` names one of `params`, its index.
fn param_index(e: &Spanned<Expr>, params: &[String]) -> Option<usize> {
    match &e.node {
        Expr::Paren(inner) => param_index(inner, params),
        Expr::Variable(n) => params.iter().position(|p| p == n),
        _ => None,
    }
}

/// Visit every statement, including nested blocks.
fn walk_stmts(stmt: &Spanned<Stmt>, f: &mut impl FnMut(&Spanned<Stmt>)) {
    f(stmt);
    match &stmt.node {
        Stmt::Block(stmts) => stmts.iter().for_each(|s| walk_stmts(s, f)),
        Stmt::If {
            then_branch,
            else_branch,
            ..
        } => {
            walk_stmts(then_branch, f);
            if let Some(e) = else_branch {
                walk_stmts(e, f);
            }
        }
        Stmt::While { body, .. }
        | Stmt::Loop { body }
        | Stmt::For { body, .. }
        | Stmt::ForEach { body, .. } => walk_stmts(body, f),
        Stmt::Match { arms, .. } => arms.iter().for_each(|a| walk_stmts(&a.body, f)),
        _ => {}
    }
}

/// Visit every expression that appears directly in this statement.
fn walk_exprs_in_stmt(stmt: &Spanned<Stmt>, f: &mut impl FnMut(&Spanned<Expr>)) {
    let mut visit = |e: &Spanned<Expr>| walk_expr(e, f);
    match &stmt.node {
        Stmt::VarDecl { init, .. } => visit(init),
        Stmt::Assign { target, value } => {
            visit(target);
            visit(value);
        }
        Stmt::Return(Some(e)) | Stmt::Expr(e) => visit(e),
        Stmt::If { condition, .. } | Stmt::While { condition, .. } => visit(condition),
        Stmt::For { range, .. } => {
            visit(&range.start);
            visit(&range.end);
        }
        Stmt::ForEach { iterable, .. } => visit(iterable),
        Stmt::Match { expr, .. } => visit(expr),
        _ => {}
    }
}

fn walk_expr(e: &Spanned<Expr>, f: &mut impl FnMut(&Spanned<Expr>)) {
    f(e);
    match &e.node {
        Expr::Paren(i) => walk_expr(i, f),
        Expr::Unary { operand, .. } => walk_expr(operand, f),
        Expr::Binary { left, right, .. } => {
            walk_expr(left, f);
            walk_expr(right, f);
        }
        Expr::Call { args, .. } => args.iter().for_each(|a| walk_expr(a, f)),
        Expr::CallIndirect { callee, args } => {
            walk_expr(callee, f);
            args.iter().for_each(|a| walk_expr(a, f));
        }
        Expr::Index { object, index } => {
            walk_expr(object, f);
            walk_expr(index, f);
        }
        Expr::Field { object, .. } => walk_expr(object, f),
        Expr::Cast { expr, .. } => walk_expr(expr, f),
        Expr::SliceLen(i) | Expr::U16Low(i) | Expr::U16High(i) => walk_expr(i, f),
        Expr::Slice {
            object, start, end, ..
        } => {
            walk_expr(object, f);
            walk_expr(start, f);
            walk_expr(end, f);
        }
        Expr::StructInit { fields, .. } | Expr::AnonStructInit { fields } => {
            fields.iter().for_each(|fi| walk_expr(&fi.value, f))
        }
        Expr::EnumVariant { data, .. } => match data {
            crate::ast::VariantData::Unit => {}
            crate::ast::VariantData::Tuple(exprs) => exprs.iter().for_each(|e| walk_expr(e, f)),
            crate::ast::VariantData::Struct(fields) => {
                fields.iter().for_each(|fi| walk_expr(&fi.value, f))
            }
        },
        Expr::Match { expr, arms } => {
            walk_expr(expr, f);
            arms.iter().for_each(|a| walk_expr(&a.body, f));
        }
        _ => {}
    }
}

/// Visit every call in this statement, with its callee name and args. Indirect
/// calls are reported under a name no function has, so the caller's
/// known-function test fails and they take the indirect path.
fn walk_calls(stmt: &Spanned<Stmt>, f: &mut impl FnMut(&str, &[Spanned<Expr>])) {
    walk_exprs_in_stmt(stmt, &mut |e| match &e.node {
        Expr::Call { function, args } => f(&function.node, args),
        Expr::CallIndirect { args, .. } => f("<indirect>", args),
        _ => {}
    });
}

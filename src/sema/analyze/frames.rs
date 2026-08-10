//! Frame finalization: static frame allocation with call-graph coloring.
//!
//! During analysis, each function's parameters and locals are assigned
//! [`SymbolLocation::FrameOffset`] offsets within a per-function frame, and the
//! call graph is recorded in `call_edges`. This pass assigns each function a
//! concrete zero-page frame base such that functions which can be simultaneously
//! active (a caller and its transitive callees) never overlap, while functions
//! that cannot (siblings in the call tree) share addresses. It then rewrites
//! every `FrameOffset` into a concrete `ZeroPage(base + offset)`.
//!
//! This is the classic 8-bit "overlay" allocation: zero runtime cost, and
//! zero-page usage bounded by the deepest call chain rather than the whole
//! program.

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use crate::codegen::memory_layout::{
    FRAME_REGION_END, FRAME_REGION_START, HARDWARE_STACK_BYTES, SOFTWARE_STACK_BYTES,
};
use crate::sema::table::SymbolLocation;
use crate::sema::{FrameInfo, InterruptSaveInfo, SemaError, Warning};

use super::SemanticAnalyzer;

/// Result of frame finalization.
pub(super) struct FinalizedFrames {
    pub frames: HashMap<String, FrameInfo>,
    pub recursive_call_edges: HashSet<(String, String)>,
    pub interrupt_save_info: HashMap<String, InterruptSaveInfo>,
}

impl SemanticAnalyzer {
    /// Assign every function a zero-page frame and rewrite all `FrameOffset`
    /// symbol locations to concrete `ZeroPage` addresses.
    ///
    /// Returns the frame map and the set of call-graph edges that lie within a
    /// recursion cycle (a call across such an edge must save/restore the callee
    /// frame at runtime).
    pub(super) fn finalize_frames(
        &mut self,
        source: &crate::ast::SourceFile,
    ) -> Result<FinalizedFrames, SemaError> {
        // Deterministic node ordering (HashMap iteration order is not stable).
        let mut nodes: Vec<String> = self.frame_sizes.keys().cloned().collect();
        nodes.sort();

        // Adjacency restricted to known functions, deduplicated and sorted.
        let mut adj: HashMap<String, Vec<String>> = HashMap::default();
        for n in &nodes {
            let mut succ: Vec<String> = self
                .call_edges
                .get(n)
                .map(|s| {
                    s.iter()
                        .filter(|c| self.frame_sizes.contains_key(*c))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            succ.sort();
            succ.dedup();
            adj.insert(n.clone(), succ);
        }

        // --- Tarjan strongly-connected components ---
        let sccs = tarjan_scc(&nodes, &adj);

        // Map each function to its SCC index.
        let mut scc_of: HashMap<String, usize> = HashMap::default();
        for (i, comp) in sccs.iter().enumerate() {
            for f in comp {
                scc_of.insert(f.clone(), i);
            }
        }

        // Recursive edges: any call edge whose endpoints share an SCC (this
        // includes direct self-recursion and mutual recursion).
        let mut recursive_call_edges: HashSet<(String, String)> = HashSet::default();
        for (caller, succ) in &adj {
            for callee in succ {
                if scc_of.get(caller) == scc_of.get(callee) {
                    recursive_call_edges.insert((caller.clone(), callee.clone()));
                }
            }
        }

        // SCC frame size = sum of member frame sizes; members laid out
        // sequentially inside the SCC block (mutually recursive functions need
        // pairwise-disjoint frames because each recursive call saves the callee
        // frame, not the caller's).
        let scc_size: Vec<u16> = sccs
            .iter()
            .map(|comp| comp.iter().map(|f| self.frame_sizes[f] as u16).sum())
            .collect();

        // Condensation edges (between distinct SCCs) and predecessor lists.
        let mut scc_preds: Vec<HashSet<usize>> = vec![HashSet::default(); sccs.len()];
        for (caller, succ) in &adj {
            let cu = scc_of[caller];
            for callee in succ {
                let cv = scc_of[callee];
                if cu != cv {
                    scc_preds[cv].insert(cu);
                }
            }
        }

        // Process SCCs in topological order (predecessors before successors).
        // Tarjan yields SCCs in reverse-topological order, so reverse it.
        let topo: Vec<usize> = (0..sccs.len()).rev().collect();

        let base_start = FRAME_REGION_START as u16;
        let mut scc_base: Vec<u16> = vec![base_start; sccs.len()];
        for &s in &topo {
            let mut base = base_start;
            for &p in &scc_preds[s] {
                base = base.max(scc_base[p] + scc_size[p]);
            }
            scc_base[s] = base;
        }

        // Assign concrete frame bases: members sequential within their SCC block.
        let mut function_frames: HashMap<String, FrameInfo> = HashMap::default();
        for (i, comp) in sccs.iter().enumerate() {
            let mut members = comp.clone();
            members.sort(); // deterministic layout
            let mut cursor = scc_base[i];
            for f in &members {
                let size = self.frame_sizes[f];
                let param_size = self
                    .function_metadata
                    .get(f)
                    .map(|m| m.param_bytes_used)
                    .unwrap_or(0);
                let end = cursor + size as u16;
                if end > FRAME_REGION_END as u16 + 1 {
                    return Err(SemaError::FrameRegionOverflow {
                        chain: describe_worst_chain(&sccs, &scc_base, &scc_size, &adj, &scc_of),
                        needed: end as usize - base_start as usize,
                    });
                }
                function_frames.insert(
                    f.clone(),
                    FrameInfo {
                        base: cursor as u8,
                        size,
                        param_size,
                    },
                );
                cursor += size as u16;
            }
        }

        let handlers: Vec<String> = self
            .function_metadata
            .iter()
            .filter(|(name, m)| m.is_interrupt_handler && self.frame_sizes.contains_key(*name))
            .map(|(name, _)| name.clone())
            .collect();

        // Interrupt-handler zero-page safety. A handler can preempt main code at
        // any point, so it must preserve every zero-page location the interrupted
        // code might be using. We save the shared codegen scratch/pools/math region
        // plus the contiguous frame span the handler's own call graph touches (its
        // frames overlap main frames under unified coloring). The handler prologue
        // (item.rs) emits the save/restore on the hardware stack.

        let mut interrupt_save_info: HashMap<String, InterruptSaveInfo> = HashMap::default();
        for handler in &handlers {
            let reachable = reachable_from(handler, &adj);

            // Recursion in interrupt context is forbidden: the software stack used
            // by frame save/restore is not reentrant with respect to preemption.
            for (caller, callee) in &recursive_call_edges {
                if reachable.contains(caller) || reachable.contains(callee) {
                    return Err(SemaError::Custom {
                        message: format!(
                            "recursion is not allowed in interrupt handler '{}' (call cycle through '{}'): \
                             the frame save/restore stack is not reentrant",
                            handler, caller
                        ),
                        span: crate::ast::Span::dummy(),
                    });
                }
            }

            // Contiguous zero-page span covering every frame the handler touches.
            let mut span_start = u16::MAX;
            let mut span_end = 0u16;
            for f in &reachable {
                if let Some(frame) = function_frames.get(f) {
                    span_start = span_start.min(frame.base as u16);
                    span_end = span_end.max(frame.base as u16 + frame.size as u16);
                }
            }
            let shared_frames = if span_end > span_start {
                vec![(span_start as u8, (span_end - span_start) as u8)]
            } else {
                Vec::new()
            };

            // Save only the scratch regions the handler's reachable code can
            // actually touch. A full scratch save is 60 zero-page bytes and
            // ~780 cycles per interrupt; a handler that does no 16-bit math
            // has no business preserving $D0-$DC.
            let (save_scratch, save_math) = self.interrupt_scratch_needs(&reachable, source);

            interrupt_save_info.insert(
                handler.clone(),
                InterruptSaveInfo {
                    save_scratch,
                    save_math,
                    shared_frames,
                },
            );
        }

        // Local-array data blocks. Same colouring rule as frames, but in RAM
        // rather than zero page, and with one extra constraint: an interrupt
        // handler is not a callee of `main`, so colouring alone would let their
        // blocks share addresses. Zero page solves that by saving the handler's
        // span on the hardware stack; a RAM block is far too large for that, so
        // the interrupt-reachable set is laid out above everything else instead.
        let mut irq_reachable: HashSet<String> = HashSet::default();
        for handler in &handlers {
            irq_reachable.extend(reachable_from(handler, &adj));
        }
        self.layout_array_blocks(&sccs, &scc_preds, &topo, &irq_reachable)?;

        self.warn_deep_recursion(&recursive_call_edges, &function_frames);
        self.warn_interrupt_stack_depth(&handlers, &adj, &interrupt_save_info, source);

        self.rewrite_frame_offsets(&function_frames)?;

        Ok(FinalizedFrames {
            frames: function_frames,
            recursive_call_edges,
            interrupt_save_info,
        })
    }

    /// Warn about recursive functions whose frame is large enough that the
    /// software stack (used to save/restore frames across recursion) allows only
    /// a shallow recursion depth before it silently overflows. Small-frame
    /// recursion is left unflagged: it is bounded instead by the ~128-level
    /// hardware-stack limit shared by all non-tail recursion, which is not
    /// frame-size-specific.
    fn warn_deep_recursion(
        &mut self,
        recursive_call_edges: &HashSet<(String, String)>,
        frames: &HashMap<String, FrameInfo>,
    ) {
        // Only flag when the estimated safe depth drops below this many levels,
        // to target genuinely risky large-frame recursion rather than every
        // small recursive helper.
        const MIN_SAFE_RECURSION_DEPTH: usize = 32;

        // Functions that participate in recursion (either endpoint of a cycle edge).
        let mut recursive_fns: Vec<String> = recursive_call_edges
            .iter()
            .flat_map(|(a, b)| [a.clone(), b.clone()])
            .collect();
        recursive_fns.sort();
        recursive_fns.dedup();

        for f in recursive_fns {
            // Tail-recursive functions are optimized into a loop (JMP, no JSR) and
            // never push their frame, so they cannot overflow the software stack.
            // (A function mixing tail and non-tail self-calls is conservatively
            // skipped too - a rare false negative, preferred over warning on the
            // common tail-recursive accumulator pattern.)
            if self
                .function_metadata
                .get(&f)
                .is_some_and(|m| m.has_tail_recursion)
            {
                continue;
            }

            let Some(frame) = frames.get(&f) else {
                continue;
            };
            if frame.size == 0 {
                continue; // No per-call software-stack cost from the frame itself.
            }
            let safe_depth = SOFTWARE_STACK_BYTES / frame.size as usize;
            if safe_depth < MIN_SAFE_RECURSION_DEPTH {
                // Attribute the warning to the function's declaration span, if known
                // (imported functions may not be in declared_functions - skip those).
                if let Some((_, span)) = self.declared_functions.iter().find(|(n, _)| *n == f) {
                    self.warnings.push(Warning::DeepRecursionRisk {
                        function: f.clone(),
                        frame_bytes: frame.size,
                        safe_depth,
                        span: *span,
                    });
                }
            }
        }
    }

    /// Warn when interrupt handlers' worst-case hardware-stack (page 1) usage
    /// leaves little headroom.
    ///
    /// A handler entry costs, on `$0100-$01FF`: 3 bytes for the CPU sequence
    /// (PCH, PCL, P), 3 for the A/X/Y prologue pushes, the zero-page save (up to
    /// ~220 bytes for a handler whose graph touches the scratch, math and frame
    /// regions), and 2 more per nested `JSR` on its deepest call chain. Handlers
    /// are summed rather than maxed: an NMI can preempt an IRQ, so both entries
    /// can be live at once.
    ///
    /// Recursion inside a handler is already a hard error (above), so every
    /// handler's subgraph is acyclic and its deepest chain is a plain memoized
    /// DFS.
    fn warn_interrupt_stack_depth(
        &mut self,
        handlers: &[String],
        adj: &HashMap<String, Vec<String>>,
        interrupt_save_info: &HashMap<String, InterruptSaveInfo>,
        source: &crate::ast::SourceFile,
    ) {
        /// Bytes the CPU pushes on interrupt entry: PCH, PCL, P.
        const CPU_ENTRY_BYTES: usize = 3;
        /// A, X, Y — three pushes on both the NMOS and CMOS prologue paths.
        const REGISTER_SAVE_BYTES: usize = 3;
        /// A `JSR` pushes a 2-byte return address.
        const JSR_BYTES: usize = 2;
        /// Warn once free space drops below this. Generous, because the true
        /// headroom is unknown: whatever the platform ROM left on the stack
        /// before `main` is already spent.
        const MIN_FREE_BYTES: usize = 32;

        if handlers.is_empty() {
            return;
        }

        // Depth through a function pointer is unknowable — `call_edges` records
        // no edge for an indirect call — so a handler that can reach one gets no
        // number rather than a wrong one.
        let indirect_risk = |reachable: &HashSet<String>| {
            reachable
                .iter()
                .any(|f| self.address_taken_functions.contains(f))
        };

        let mut per_handler: Vec<(String, usize, usize)> = Vec::new();
        let mut total = 0usize;
        for handler in handlers {
            let reachable = reachable_from(handler, adj);
            if indirect_risk(&reachable) {
                return;
            }
            let call_depth = self.max_call_depth(handler, adj, &mut HashMap::default());
            let entry_bytes = CPU_ENTRY_BYTES
                + REGISTER_SAVE_BYTES
                + interrupt_save_info
                    .get(handler)
                    .map_or(0, |si| si.zp_save_bytes());
            total += entry_bytes + call_depth * JSR_BYTES;
            per_handler.push((handler.clone(), entry_bytes, call_depth));
        }

        if total + MIN_FREE_BYTES <= HARDWARE_STACK_BYTES {
            return;
        }

        // Attribute to the costliest handler. Its span comes from the AST rather
        // than `declared_functions`, which deliberately omits `#[irq]`/`#[nmi]`
        // (they are never "unused"). A handler defined in an imported module is
        // not in this `source` and is skipped, matching `warn_deep_recursion`.
        per_handler.sort_by_key(|(_, entry, depth)| entry + depth * JSR_BYTES);
        if let Some((handler, entry_bytes, call_depth)) = per_handler.last()
            && let Some(span) = source.items.iter().find_map(|item| match &item.node {
                crate::ast::Item::Function(f) if f.name.node == *handler => Some(f.name.span),
                _ => None,
            })
        {
            self.warnings.push(Warning::InterruptStackDepth {
                handler: handler.clone(),
                entry_bytes: *entry_bytes,
                call_depth: *call_depth,
                total_bytes: total,
                span,
            });
        }
    }

    /// Deepest chain of nested `JSR`s reachable from `f`, memoized.
    ///
    /// Inline functions emit no `JSR`, so they cost nothing themselves but still
    /// contribute their callees. Tail-recursive self-calls compile to `JMP` and
    /// are likewise free — the same exemption `warn_deep_recursion` applies. The
    /// caller guarantees an acyclic subgraph, but `visiting` still guards against
    /// a cycle so a bug elsewhere cannot hang the compiler.
    fn max_call_depth(
        &self,
        f: &str,
        adj: &HashMap<String, Vec<String>>,
        memo: &mut HashMap<String, usize>,
    ) -> usize {
        if let Some(d) = memo.get(f) {
            return *d;
        }
        // Mark in-progress so a cycle resolves to 0 rather than recursing forever.
        memo.insert(f.to_string(), 0);
        let deepest = adj
            .get(f)
            .map(|callees| {
                callees
                    .iter()
                    .filter(|c| c.as_str() != f) // self-recursion: JMP, not JSR
                    .map(|c| {
                        let below = self.max_call_depth(c, adj, memo);
                        let is_inline = self.function_metadata.get(c).is_some_and(|m| m.is_inline);
                        if is_inline { below } else { below + 1 }
                    })
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        memo.insert(f.to_string(), deepest);
        deepest
    }

    /// Rewrite every `FrameOffset(off)` into `ZeroPage(base + off)` across all
    /// stored symbol locations, and rebase the offset-valued metadata maps.
    /// Lay out each function's local-array data block in the BSS section.
    ///
    /// The rule is the frame-colouring rule: an SCC starts above the end of
    /// every predecessor (caller) SCC, so a callee's arrays never overlap a live
    /// caller's. Unrelated functions share addresses, which is what keeps a
    /// 1 KB BSS viable.
    ///
    /// Interrupt-reachable functions are laid out above the main-thread
    /// high-water mark. A handler is not a callee of `main`, so colouring alone
    /// would let their blocks overlap; zero-page frames survive that by having
    /// the handler save the span it touches, but a RAM block is far too large to
    /// push onto the hardware stack.
    fn layout_array_blocks(
        &mut self,
        sccs: &[Vec<String>],
        scc_preds: &[HashSet<usize>],
        topo: &[usize],
        irq_reachable: &HashSet<String>,
    ) -> Result<(), SemaError> {
        if self.array_block_sizes.is_empty() {
            return Ok(());
        }

        const DEFAULT_BSS: (u16, u16) = (0x0400, 0x07FF);
        let (bss_start, bss_end) = self
            .memory_config
            .get_section("BSS")
            .map(|s| (s.start, s.end))
            .unwrap_or(DEFAULT_BSS);
        // Statics were allocated first and are never reused, so blocks begin
        // above whatever they consumed.
        let base_start = self.import_context.borrow().bss_cursor.unwrap_or(bss_start);

        let size_of = |f: &String| self.array_block_sizes.get(f).copied().unwrap_or(0);
        let scc_size: Vec<u16> = sccs.iter().map(|c| c.iter().map(&size_of).sum()).collect();
        let scc_is_irq: Vec<bool> = sccs
            .iter()
            .map(|c| c.iter().any(|f| irq_reachable.contains(f)))
            .collect();

        // Pass A: main-thread SCCs.
        let mut scc_base: Vec<u16> = vec![base_start; sccs.len()];
        let mut main_hwm = base_start;
        for &i in topo {
            if scc_is_irq[i] {
                continue;
            }
            let mut base = base_start;
            for &p in &scc_preds[i] {
                if !scc_is_irq[p] {
                    base = base.max(scc_base[p] + scc_size[p]);
                }
            }
            scc_base[i] = base;
            main_hwm = main_hwm.max(base + scc_size[i]);
        }

        // Pass B: interrupt-reachable SCCs, above everything the main thread
        // can be holding when a handler fires.
        for &i in topo {
            if !scc_is_irq[i] {
                continue;
            }
            let mut base = main_hwm;
            for &p in &scc_preds[i] {
                base = base.max(scc_base[p] + scc_size[p]);
            }
            scc_base[i] = base;
        }

        // Members are laid out sequentially inside their SCC block: mutually
        // recursive functions are simultaneously live, so they cannot share.
        let mut function_base: HashMap<String, u16> = HashMap::default();
        for (i, comp) in sccs.iter().enumerate() {
            let mut members = comp.clone();
            members.sort(); // deterministic layout
            let mut cursor = scc_base[i];
            for f in &members {
                let size = size_of(f);
                if size > 0 && (cursor as u32 + size as u32 - 1) > bss_end as u32 {
                    return Err(SemaError::Custom {
                        message: format!(
                            "BSS section overflow: local array data for '{}' does not fit in \
                             ${:04X}-${:04X}. Arrays are colored by the call graph, so this is \
                             the deepest chain, not the total; move a large buffer to a `static` \
                             or enlarge the BSS section in wraith.toml",
                            f, bss_start, bss_end
                        ),
                        span: crate::ast::Span::dummy(),
                    });
                }
                function_base.insert(f.clone(), cursor);
                cursor += size;
            }
        }

        // Rebase every declaration's offset to an absolute address.
        for arr in self
            .local_arrays
            .values_mut()
            .chain(self.enum_blocks.values_mut())
            .chain(self.string_buffers.values_mut())
            .chain(self.struct_temps.values_mut())
        {
            let base = function_base
                .get(&arr.function)
                .copied()
                .unwrap_or(base_start);
            arr.addr += base;
        }

        Ok(())
    }

    /// Whether a handler's reachable code can touch the shared codegen
    /// scratch region ($20-$3F / $E0-$FE) or the 16-bit math routines'
    /// working storage ($D0-$DC), as (scratch, math). Anything not provably
    /// trivial sets scratch; only a 16-bit Mul/Div/Mod (or a direct call to
    /// those routines) sets math.
    fn interrupt_scratch_needs(
        &self,
        reachable: &HashSet<String>,
        source: &crate::ast::SourceFile,
    ) -> (bool, bool) {
        let mut needs = (false, false);
        for item in source.items.iter().chain(self.imported_items.iter()) {
            let crate::ast::Item::Function(f) = &item.node else {
                continue;
            };
            if reachable.contains(&f.name.node) {
                self.scan_stmt_for_scratch(&f.body.node, &mut needs);
            }
            if needs.0 && needs.1 {
                break;
            }
        }
        needs
    }

    fn scan_stmt_for_scratch(&self, stmt: &crate::ast::Stmt, needs: &mut (bool, bool)) {
        use crate::ast::Stmt;
        match stmt {
            Stmt::VarDecl { init, .. } => self.scan_expr_for_scratch(init, needs),
            Stmt::Assign { target, value } => {
                self.scan_expr_for_scratch(target, needs);
                self.scan_expr_for_scratch(value, needs);
            }
            Stmt::Expr(e) => self.scan_expr_for_scratch(e, needs),
            Stmt::Return(Some(e)) => self.scan_expr_for_scratch(e, needs),
            Stmt::Return(None) | Stmt::Break | Stmt::Continue => {}
            Stmt::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.scan_expr_for_scratch(condition, needs);
                self.scan_stmt_for_scratch(&then_branch.node, needs);
                if let Some(e) = else_branch {
                    self.scan_stmt_for_scratch(&e.node, needs);
                }
            }
            Stmt::While { condition, body } => {
                self.scan_expr_for_scratch(condition, needs);
                self.scan_stmt_for_scratch(&body.node, needs);
            }
            Stmt::Loop { body } => self.scan_stmt_for_scratch(&body.node, needs),
            Stmt::For { range, body, .. } => {
                self.scan_expr_for_scratch(&range.start, needs);
                self.scan_expr_for_scratch(&range.end, needs);
                self.scan_stmt_for_scratch(&body.node, needs);
            }
            Stmt::ForEach { iterable, body, .. } => {
                needs.0 = true; // the loop stages a pointer for its whole body
                self.scan_expr_for_scratch(iterable, needs);
                self.scan_stmt_for_scratch(&body.node, needs);
            }
            Stmt::Match { expr, arms } => {
                needs.0 = true; // dispatch scratch + scrutinee staging
                self.scan_expr_for_scratch(expr, needs);
                for arm in arms {
                    self.scan_stmt_for_scratch(&arm.body.node, needs);
                }
            }
            Stmt::Block(stmts) => {
                for s in stmts {
                    self.scan_stmt_for_scratch(&s.node, needs);
                }
            }
            // Hand-written assembly may use any zero-page byte.
            Stmt::Asm { .. } => needs.0 = true,
        }
    }

    fn scan_expr_for_scratch(
        &self,
        expr: &crate::ast::Spanned<crate::ast::Expr>,
        needs: &mut (bool, bool),
    ) {
        use crate::ast::{BinaryOp, Expr, Literal};
        match &expr.node {
            // Loads that compile to a single instruction: no scratch involved.
            Expr::Literal(Literal::Integer(_))
            | Expr::Literal(Literal::Bool(_))
            | Expr::Variable(_)
            | Expr::CpuFlagCarry
            | Expr::CpuFlagZero
            | Expr::CpuFlagOverflow
            | Expr::CpuFlagNegative => {}

            Expr::Paren(inner) => self.scan_expr_for_scratch(inner, needs),

            // A bit op is a load/AND/store (or a single SMB/RMB) — no scratch
            // pool of its own; only its subexpressions might use one.
            Expr::BitOp { object, bit, .. } => {
                self.scan_expr_for_scratch(object, needs);
                self.scan_expr_for_scratch(bit, needs);
            }

            Expr::Binary { op, left, right } => {
                needs.0 = true; // operand staging ($F0 pool / $20)
                if matches!(op, BinaryOp::Mul | BinaryOp::Div | BinaryOp::Mod) {
                    let wide = self.resolved_types.get(&expr.span).is_some_and(|t| {
                        matches!(
                            t,
                            crate::sema::types::Type::Primitive(
                                crate::ast::PrimitiveType::U16
                                    | crate::ast::PrimitiveType::I16
                                    | crate::ast::PrimitiveType::B16
                            )
                        )
                    });
                    if wide {
                        needs.1 = true; // JSR mul16/div16/mod16 uses $D0-$DC
                    }
                }
                self.scan_expr_for_scratch(left, needs);
                self.scan_expr_for_scratch(right, needs);
            }

            Expr::Call { function, args } => {
                needs.0 = true; // argument staging
                if matches!(function.node.as_str(), "mul16" | "div16" | "mod16") {
                    needs.1 = true;
                }
                for a in args {
                    self.scan_expr_for_scratch(a, needs);
                }
            }

            // Everything else — calls through pointers, casts, aggregates,
            // slices, matches, strings — stages through the scratch pools.
            Expr::Literal(_)
            | Expr::Unary { .. }
            | Expr::Cast { .. }
            | Expr::Field { .. }
            | Expr::Index { .. }
            | Expr::Slice { .. }
            | Expr::CallIndirect { .. }
            | Expr::StructInit { .. }
            | Expr::AnonStructInit { .. }
            | Expr::EnumVariant { .. }
            | Expr::SliceLen(_)
            | Expr::U16Low(_)
            | Expr::U16High(_)
            | Expr::Match { .. } => needs.0 = true,
        }
    }

    fn rewrite_frame_offsets(
        &mut self,
        frames: &HashMap<String, FrameInfo>,
    ) -> Result<(), SemaError> {
        // resolved_symbols: the store codegen reads for variable addresses
        // (one entry per declaration and per use).
        for info in self.resolved_symbols.values_mut() {
            if let SymbolLocation::FrameOffset(off) = info.location {
                let base = frame_base(frames, info.containing_function.as_deref())?;
                info.location = SymbolLocation::ZeroPage(base + off);
            }
        }

        // Hidden loop-bound slots: same rebasing as ordinary symbols.
        for info in self.loop_bound_slots.values_mut() {
            if let SymbolLocation::FrameOffset(off) = info.location {
                let base = frame_base(frames, info.containing_function.as_deref())?;
                info.location = SymbolLocation::ZeroPage(base + off);
            }
        }

        // Function metadata: inline param symbols hold frame offsets that must be
        // rebased for cross-module inline expansion.
        for meta in self.function_metadata.values_mut() {
            if let Some(syms) = meta.inline_param_symbols.as_mut() {
                for info in syms.values_mut() {
                    if let SymbolLocation::FrameOffset(off) = info.location {
                        let base = frame_base(frames, info.containing_function.as_deref())?;
                        info.location = SymbolLocation::ZeroPage(base + off);
                    }
                }
            }
        }

        Ok(())
    }
}

/// Resolve a function's frame base, mapping missing/None cases to an internal error.
fn frame_base(frames: &HashMap<String, FrameInfo>, func: Option<&str>) -> Result<u8, SemaError> {
    let name = func.ok_or_else(|| SemaError::Custom {
        message: "internal: FrameOffset symbol without a containing function".to_string(),
        span: crate::ast::Span::dummy(),
    })?;
    frames
        .get(name)
        .map(|f| f.base)
        .ok_or_else(|| SemaError::Custom {
            message: format!("internal: no frame assigned for function '{}'", name),
            span: crate::ast::Span::dummy(),
        })
}

/// Set of functions reachable from `start` (inclusive) over the call graph.
fn reachable_from(start: &str, adj: &HashMap<String, Vec<String>>) -> HashSet<String> {
    let mut seen: HashSet<String> = HashSet::default();
    let mut stack = vec![start.to_string()];
    while let Some(n) = stack.pop() {
        if !seen.insert(n.clone()) {
            continue;
        }
        if let Some(succ) = adj.get(&n) {
            for s in succ {
                if !seen.contains(s) {
                    stack.push(s.clone());
                }
            }
        }
    }
    seen
}

/// Recursive Tarjan SCC. Returns components in reverse-topological order
/// (each component before its predecessors), matching the classic algorithm.
fn tarjan_scc(nodes: &[String], adj: &HashMap<String, Vec<String>>) -> Vec<Vec<String>> {
    struct State<'a> {
        adj: &'a HashMap<String, Vec<String>>,
        index: usize,
        indices: HashMap<String, usize>,
        low: HashMap<String, usize>,
        on_stack: HashSet<String>,
        stack: Vec<String>,
        out: Vec<Vec<String>>,
    }

    fn strong_connect(st: &mut State, v: &str) {
        st.indices.insert(v.to_string(), st.index);
        st.low.insert(v.to_string(), st.index);
        st.index += 1;
        st.stack.push(v.to_string());
        st.on_stack.insert(v.to_string());

        if let Some(succ) = st.adj.get(v) {
            for w in succ {
                if !st.indices.contains_key(w) {
                    strong_connect(st, w);
                    let lw = st.low[w];
                    let lv = st.low[v];
                    st.low.insert(v.to_string(), lv.min(lw));
                } else if st.on_stack.contains(w) {
                    let iw = st.indices[w];
                    let lv = st.low[v];
                    st.low.insert(v.to_string(), lv.min(iw));
                }
            }
        }

        if st.low[v] == st.indices[v] {
            let mut comp = Vec::new();
            loop {
                let w = st.stack.pop().unwrap();
                st.on_stack.remove(&w);
                let done = w == v;
                comp.push(w);
                if done {
                    break;
                }
            }
            st.out.push(comp);
        }
    }

    let mut st = State {
        adj,
        index: 0,
        indices: HashMap::default(),
        low: HashMap::default(),
        on_stack: HashSet::default(),
        stack: Vec::new(),
        out: Vec::new(),
    };
    for n in nodes {
        if !st.indices.contains_key(n) {
            strong_connect(&mut st, n);
        }
    }
    st.out
}

/// Build a human-readable description of the deepest (highest-ending) call chain,
/// for the frame-overflow diagnostic.
fn describe_worst_chain(
    sccs: &[Vec<String>],
    scc_base: &[u16],
    scc_size: &[u16],
    _adj: &HashMap<String, Vec<String>>,
    _scc_of: &HashMap<String, usize>,
) -> String {
    // Report the SCC with the highest end address and its cumulative usage.
    let mut worst = 0usize;
    let mut worst_end = 0u16;
    for i in 0..sccs.len() {
        let end = scc_base[i] + scc_size[i];
        if end > worst_end {
            worst_end = end;
            worst = i;
        }
    }
    let mut names = sccs[worst].clone();
    names.sort();
    format!(
        "deepest frames end at ${:02X} (functions: {})",
        worst_end.saturating_sub(1),
        names.join(", ")
    )
}

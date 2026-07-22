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

use crate::codegen::memory_layout::{FRAME_REGION_END, FRAME_REGION_START, SOFTWARE_STACK_BYTES};
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
    pub(super) fn finalize_frames(&mut self) -> Result<FinalizedFrames, SemaError> {
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

        // Interrupt-handler zero-page safety. A handler can preempt main code at
        // any point, so it must preserve every zero-page location the interrupted
        // code might be using. We save the shared codegen scratch/pools/math region
        // plus the contiguous frame span the handler's own call graph touches (its
        // frames overlap main frames under unified coloring). The handler prologue
        // (item.rs) emits the save/restore on the hardware stack.
        let handlers: Vec<String> = self
            .function_metadata
            .iter()
            .filter(|(name, m)| m.is_interrupt_handler && self.frame_sizes.contains_key(*name))
            .map(|(name, _)| name.clone())
            .collect();

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
                        span: crate::ast::Span { start: 0, end: 0 },
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

            interrupt_save_info.insert(
                handler.clone(),
                InterruptSaveInfo {
                    save_scratch: true,
                    save_math: true,
                    shared_frames,
                },
            );
        }

        self.warn_deep_recursion(&recursive_call_edges, &function_frames);

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

    /// Rewrite every `FrameOffset(off)` into `ZeroPage(base + off)` across all
    /// stored symbol locations, and rebase the offset-valued metadata maps.
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
        span: crate::ast::Span { start: 0, end: 0 },
    })?;
    frames
        .get(name)
        .map(|f| f.base)
        .ok_or_else(|| SemaError::Custom {
            message: format!("internal: no frame assigned for function '{}'", name),
            span: crate::ast::Span { start: 0, end: 0 },
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

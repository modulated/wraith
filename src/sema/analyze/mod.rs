//! Semantic Analysis Logic
//!
//! Traverses the AST to populate the symbol table and perform type checking.

pub(crate) mod escape;
mod expr;
mod frames;
mod register;
mod stmt;
mod tail_call;
mod unused;

use crate::ast::{Function, Item, PrimitiveType, SourceFile, Spanned, TypeExpr};
use crate::sema::const_eval::ConstEnv;
use crate::sema::table::{SymbolInfo, SymbolKind, SymbolLocation, SymbolTable};
use crate::sema::type_defs::TypeRegistry;
use crate::sema::types::Type;
use crate::sema::{FunctionMetadata, ProgramInfo, SemaError, Warning};

use crate::ast::Span;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

/// State shared by every analyzer working on one compilation's import graph.
///
/// A module is analyzed once and *replayed* into every later importer: the
/// first import of a file runs the full merge; a later one — a diamond, like
/// A importing B and C while C also imports B — re-merges the stored analysis
/// instead of skipping the file and losing its symbols (which used to
/// produce "undefined symbol" in the second importer). A path on the stack is
/// a genuine cycle and is an error.
///
/// The BSS cursor is shared as well, so mutable statics from every module
/// come from one global address space and never collide, regardless of
/// declaration or import order.
pub struct ImportContext {
    /// Every fully analyzed module, by canonical path, with its items (needed
    /// to replay the codegen item list without re-reading the file).
    modules: HashMap<PathBuf, (Rc<SemanticAnalyzer>, Vec<Spanned<Item>>)>,
    /// The import chain from the root to the module being analyzed. A path
    /// already here is a true cycle (A -> B -> A).
    stack: Vec<PathBuf>,
    /// Global BSS high-water mark for mutable statics (see `bss_alloc`).
    bss_cursor: Option<u16>,
}

impl ImportContext {
    fn new() -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            modules: HashMap::default(),
            stack: Vec::new(),
            bss_cursor: None,
        }))
    }
}

/// Adding a field? Decide how it crosses an import in
/// `merge_imported` (register.rs) — analyzer state an imported function's
/// codegen needs but that nobody merged has been a recurring bug class.
pub struct SemanticAnalyzer {
    pub table: SymbolTable,
    pub warnings: Vec<Warning>,
    /// Errors collected so analysis can report several per run instead of
    /// stopping at the first, mirroring the parser's `errors` field. Analysis
    /// still fails if this is non-empty — nothing partial reaches codegen.
    pub errors: Vec<SemaError>,
    pub(super) current_return_type: Option<Type>,
    pub(super) resolved_symbols: HashMap<Span, SymbolInfo>,
    pub(super) function_metadata: HashMap<String, FunctionMetadata>,
    pub(super) folded_constants: HashMap<Span, crate::sema::const_eval::ConstValue>,
    /// Hidden per-loop frame slots holding non-constant for-loop range ends,
    /// keyed by the range end expression's span. Frame-allocated so nested
    /// loops, scratch-using expressions, and calls in the body cannot clobber
    /// a live bound.
    pub(super) loop_bound_slots: HashMap<Span, SymbolInfo>,
    /// Where each local array's *data* lives, keyed by the declaration's name
    /// span. During analysis this holds an offset within the declaring
    /// function's array block; `finalize_frames` rewrites it to an absolute RAM
    /// address once the blocks have been laid out.
    pub(super) local_arrays: HashMap<Span, crate::sema::LocalArray>,
    /// Where each enum-typed local's *data* lives, same keying and rebasing as
    /// `local_arrays`. An enum value binds by copying the constructed bytes
    /// into this block; without it every variable points into shared codegen
    /// scratch and two live enums alias.
    pub(super) enum_blocks: HashMap<Span, crate::sema::LocalArray>,
    /// Where each `str<N>` buffer's `[len][bytes]` block lives, keyed and
    /// rebased exactly like `local_arrays`. `size` is the whole block
    /// (`1 + capacity`); the buffer's capacity is `size - 1`. Membership also
    /// marks the declaration as a *writable* string, which is what lets
    /// `s[i] = c` past the "can't write a str literal" guard.
    pub(super) string_buffers: HashMap<Span, crate::sema::LocalArray>,
    /// Bytes of local-array data each function needs, consumed by
    /// `finalize_frames` to lay the blocks out in RAM.
    pub(super) array_block_sizes: HashMap<String, u16>,
    /// Bump cursor for the current function's array block, reset per function
    /// alongside `frame_cursor`.
    pub(super) array_cursor: u16,
    pub(super) resolved_types: HashMap<Span, Type>,
    pub(super) type_registry: TypeRegistry,
    pub(super) imported_items: Vec<Spanned<Item>>,
    pub(super) base_path: Option<PathBuf>,
    pub(super) import_context: Rc<RefCell<ImportContext>>,
    pub(super) const_env: ConstEnv,
    pub(super) loop_depth: usize,
    /// Track variable usage for unused variable warnings (per-function, cleared after each function)
    pub(super) used_variables: HashSet<String>,
    /// Track ALL symbol usage across entire file (never cleared, for import checking)
    pub(super) all_used_symbols: HashSet<String>,
    /// Track declared variables in current scope (name -> span) for unused variable detection
    pub(super) declared_variables: Vec<(String, Span)>,
    /// Track function parameters (name -> span) for unused parameter detection
    pub(super) declared_parameters: Vec<(String, Span)>,
    /// Track imported symbols (name -> span) for unused import detection
    pub(super) imported_symbols: Vec<(String, Span)>,
    /// Track declared functions (name -> span) for unused function detection
    pub(super) declared_functions: Vec<(String, Span)>,
    /// Track function calls for unused function detection
    pub(super) called_functions: HashSet<String>,
    /// Track unreachable statements for dead code elimination
    pub(super) unreachable_stmts: HashSet<Span>,
    /// True when checking an assignment target (not reading a value)
    pub(super) checking_assignment_target: bool,
    /// Expected type for type inference (e.g., for anonymous struct literals)
    pub(super) expected_type: Option<Type>,
    /// Map from span to resolved struct name for anonymous struct inits
    pub(super) resolved_struct_names: HashMap<Span, String>,
    /// Spans of `.len`/`.low`/`.high` expressions resolved as struct field
    /// accesses rather than the built-in accessors (see `ProgramInfo::
    /// accessor_fields`).
    pub(super) accessor_fields: HashSet<Span>,
    /// Memory configuration from wraith.toml for overlap checking
    pub(super) memory_config: crate::config::MemoryConfig,
    /// Current function being analyzed (for tracking symbol scope in inline asm)
    pub(super) current_function: Option<String>,
    /// Bump cursor for the current function's frame (offset from frame base).
    /// Reset to 0 at the start of each function; params then locals allocate upward.
    pub(super) frame_cursor: u8,
    /// Released loop-bound slots (offset, size) available for reuse within the
    /// current function. Sibling loops are never live at the same time, so
    /// their hidden bound slots can share zero-page bytes; nested loops find
    /// the list empty (the outer slot is still held) and allocate fresh.
    pub(super) loop_bound_free: Vec<(u8, u8)>,
    /// Startup values for mutable statics, in declaration order.
    pub(super) static_inits: Vec<crate::sema::StaticInit>,
    /// Per-function frame size in bytes (params + locals), the high-water mark of
    /// `frame_cursor` after analyzing each function. Consumed by `finalize_frames`.
    pub(super) frame_sizes: HashMap<String, u8>,
    /// Signatures (Type::Function) of every function reachable in codegen, keyed
    /// by name — including functions from imported modules that this module did
    /// not name explicitly. Codegen consults this to marshal call arguments when
    /// the symbol table has no entry (e.g. one imported function calling another).
    pub(super) function_signatures: HashMap<String, Type>,
    /// Call graph edges: caller name -> set of callee names. Built during body
    /// analysis (direct calls and inline calls) and consumed by `finalize_frames`
    /// to color frames and detect recursion.
    pub(super) call_edges: HashMap<String, HashSet<String>>,
    /// Functions whose address is taken (used as a value / function pointer).
    /// These receive arguments through the fixed indirect-arg staging block so
    /// an indirect caller (which cannot know the callee's colored frame) can
    /// still pass args; their prologue copies staging -> frame params.
    pub(super) address_taken_functions: HashSet<String>,
    /// Every symbol each function references: calls, constants, statics,
    /// addresses and inline-asm operands. A superset of `call_edges`, used to
    /// decide which imported items are live (see `reachable_symbols`).
    /// References made outside any function body (a `static`'s initializer, for
    /// instance) are keyed under `None`.
    pub(super) symbol_refs: HashMap<Option<String>, HashSet<String>>,
}

impl Default for SemanticAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl SemanticAnalyzer {
    pub fn new() -> Self {
        Self {
            table: SymbolTable::new(),
            warnings: Vec::with_capacity(16),
            errors: Vec::new(),
            current_return_type: None,
            resolved_symbols: HashMap::default(),
            function_metadata: HashMap::default(),
            folded_constants: HashMap::default(),
            loop_bound_slots: HashMap::default(),
            local_arrays: HashMap::default(),
            enum_blocks: HashMap::default(),
            string_buffers: HashMap::default(),
            array_block_sizes: HashMap::default(),
            array_cursor: 0,
            resolved_types: HashMap::default(),
            type_registry: TypeRegistry::new(),
            imported_items: Vec::with_capacity(8),
            base_path: None,
            import_context: ImportContext::new(),
            const_env: ConstEnv::default(),
            loop_depth: 0,
            used_variables: HashSet::default(),
            all_used_symbols: HashSet::default(),
            declared_variables: Vec::with_capacity(16),
            declared_parameters: Vec::with_capacity(8),
            imported_symbols: Vec::with_capacity(8),
            declared_functions: Vec::with_capacity(16),
            called_functions: HashSet::default(),
            unreachable_stmts: HashSet::default(),
            checking_assignment_target: false,
            expected_type: None,
            resolved_struct_names: HashMap::default(),
            accessor_fields: HashSet::default(),
            memory_config: crate::config::MemoryConfig::load_or_default(),
            current_function: None,
            frame_cursor: 0,
            loop_bound_free: Vec::new(),
            static_inits: Vec::new(),
            frame_sizes: HashMap::default(),
            function_signatures: HashMap::default(),
            call_edges: HashMap::default(),
            address_taken_functions: HashSet::default(),
            symbol_refs: HashMap::default(),
        }
    }

    pub fn with_base_path(base_path: PathBuf) -> Self {
        // One field off the default; duplicating the whole initializer meant
        // every new field had to be added twice (and once wasn't, forever).
        let mut analyzer = Self::new();
        analyzer.base_path = Some(base_path);
        analyzer
    }

    /// Get the standard library path
    /// Checks WRAITH_STD_PATH environment variable, falls back to ./std
    pub(super) fn get_std_lib_path() -> PathBuf {
        std::env::var("WRAITH_STD_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("std"))
    }

    /// Compute the size of a type, looking up named types in the registry
    /// Does `ty` contain the struct `target` *by value* — directly, through an
    /// array element, or through another by-value struct field?
    ///
    /// A pointer, slice, string, or function pointer breaks the chain (it is a
    /// fixed 2/4-byte handle, not inline storage), so `&target` is fine. Used to
    /// reject a struct that contains itself, which has no finite size and whose
    /// self-field otherwise silently sizes to 0 (the name is not yet registered,
    /// so `type_size` returns its unknown-type fallback).
    pub(super) fn type_reaches(
        &self,
        ty: &Type,
        target: &str,
        visited: &mut HashSet<String>,
    ) -> bool {
        match ty {
            Type::Named(inner) => {
                if inner == target {
                    return true;
                }
                if visited.insert(inner.clone())
                    && let Some(def) = self.type_registry.structs.get(inner)
                {
                    return def
                        .fields
                        .iter()
                        .any(|f| self.type_reaches(&f.ty, target, visited));
                }
                false
            }
            Type::Array(element, _) => self.type_reaches(element, target, visited),
            // Pointer/Slice/String/Function are fixed-size handles; primitives
            // and Void cannot contain a struct.
            _ => false,
        }
    }

    pub(super) fn type_size(&self, ty: &Type) -> usize {
        match ty {
            Type::Primitive(prim) => prim.size_bytes(),
            Type::Array(element_ty, len) => self.type_size(element_ty) * len,
            Type::Slice(_) => 4, // Fat pointer: 2 bytes base address + 2 bytes length
            Type::Pointer(_) => 2, // 16-bit address; never recurse into the pointee
            Type::String => 2,   // String is represented as a pointer
            Type::Function(_, _) => 2, // Function pointer is 16-bit
            Type::Void => 0,
            Type::Error => 0, // Poison; analysis has already failed
            Type::Named(name) => {
                // Look up in struct registry
                if let Some(struct_def) = self.type_registry.structs.get(name) {
                    return struct_def.total_size;
                }
                // Look up in enum registry
                if let Some(enum_def) = self.type_registry.enums.get(name) {
                    return enum_def.total_size;
                }
                // Unknown type - return 0 as fallback
                0
            }
        }
    }

    /// Analyze a module without finalizing frames.
    ///
    /// This runs registration, body analysis, unused checks, and tail-call
    /// analysis, leaving every parameter/local at a `FrameOffset` and the call
    /// graph in `call_edges`. Frame finalization is deferred to the root
    /// analyzer so that imported functions are colored together with the main
    /// module (a child module must NOT assign its own frame bases). Returns the
    /// tail-call info for the module.
    pub(super) fn analyze_module(
        &mut self,
        source: &SourceFile,
    ) -> Result<HashMap<String, crate::sema::TailCallInfo>, SemaError> {
        // First pass: Register all global items (functions, statics, structs).
        // Declarations are independent of each other, so all of their problems
        // are reported together.
        for item in &source.items {
            if let Err(e) = self.register_item(item) {
                self.record(e);
            }
        }

        // But stop before the bodies. A declaration that failed to register left
        // its symbol missing, so every use of it below would report "cannot find
        // ..." on top of the real error — symptoms burying the cause. Getting
        // both passes in one run would need per-name suppression of exactly
        // those follow-ons; reporting every declaration problem at once, and
        // bodies once the declarations are sound, buys most of the benefit with
        // none of the risk.
        if let Some(e) = self.take_errors() {
            return Err(e);
        }

        // Second pass: Analyze function bodies
        for item in &source.items {
            self.analyze_item(item)?;
        }

        // Surface whatever the two passes collected before going further. This
        // has to live here, not in `analyze`: an imported module is analyzed
        // through `analyze_module` too, and its errors must reach the importer
        // rather than being left in a child analyzer nobody inspects.
        //
        // The passes below are skipped on failure deliberately — unused-import
        // detection and tail-call analysis read state that is incomplete for a
        // function that failed, and would only add follow-on noise.
        if let Some(e) = self.take_errors() {
            return Err(e);
        }

        // Check for unused imports after all analysis is complete. Unused
        // *functions* are reported from `analyze` instead: whether one is dead
        // is a whole-program question, and only the root sees the whole
        // program.
        self.check_unused_imports();

        // Analyze tail calls after all other analysis is complete
        Ok(self.analyze_tail_calls(source))
    }

    /// The most errors one analysis collects before it stops trying. Matches
    /// the parser's ceiling: recovery makes long cascades rare, but a
    /// pathological file still gets a bound.
    pub(super) const MAX_ERRORS: usize = 50;

    /// Record an error and continue analyzing.
    ///
    /// Only call this where the remaining work is genuinely independent of what
    /// failed — recovering into state the failure invalidated is how a
    /// diagnostics feature turns into a miscompile.
    pub(super) fn record(&mut self, error: SemaError) {
        if self.errors.len() < Self::MAX_ERRORS {
            self.errors.push(error);
        }
    }

    /// Record a failed subexpression and yield the poison type in its place.
    ///
    /// Only for positions whose siblings are genuinely independent — each
    /// argument of a call, each operand of a binary op — so that two mistakes in
    /// one expression both report. The `Type::Error` result keeps every
    /// downstream check quiet about the value it could not determine.
    pub(super) fn poison_on_err(&mut self, result: Result<Type, SemaError>) -> Type {
        match result {
            Ok(t) => t,
            Err(e) => {
                self.record(e);
                Type::Error
            }
        }
    }

    /// Collapse collected errors into the single value `analyze` returns.
    /// One error is returned as itself so existing single-error diagnostics —
    /// and the tests pinning them — are unchanged.
    pub(super) fn take_errors(&mut self) -> Option<SemaError> {
        match self.errors.len() {
            0 => None,
            1 => Some(self.errors.remove(0)),
            _ => Some(SemaError::Multiple(std::mem::take(&mut self.errors))),
        }
    }

    pub fn analyze(&mut self, source: &SourceFile) -> Result<ProgramInfo, SemaError> {
        // `analyze_module` surfaces anything its two passes collected, so the
        // whole-program phases below only ever run on a clean module.
        let tail_call_info = self.analyze_module(source)?;

        // What the output actually needs. Computed here, at the root, because
        // only the root sees the merged reference graph. Everything outside
        // this set is dropped by codegen; warn about the root module's share of
        // it so the report and the output agree.
        let reachable_symbols = self.reachable_symbols(source);
        self.warn_unreachable_items(source, &reachable_symbols);

        // Finalize frames once, over the merged program (main module plus every
        // imported module whose call graph and frame sizes were merged in during
        // import processing). This assigns concrete zero-page frame bases and
        // rewrites all FrameOffset locations to ZeroPage.
        let finalized = self.finalize_frames(source)?;
        let function_frames = finalized.frames;
        let recursive_call_edges = finalized.recursive_call_edges;

        // Pointer escape rules. After `finalize_frames`, which is what supplies
        // the recursion cycles, and over the whole program because two of the
        // rules need information no single body has.
        self.check_pointer_escapes(source, &recursive_call_edges)?;
        let interrupt_save_info = finalized.interrupt_save_info;

        Ok(ProgramInfo {
            table: self.table.clone(),
            resolved_symbols: self.resolved_symbols.clone(),
            function_metadata: self.function_metadata.clone(),
            folded_constants: self.folded_constants.clone(),
            loop_bound_slots: self.loop_bound_slots.clone(),
            local_arrays: self.local_arrays.clone(),
            enum_blocks: self.enum_blocks.clone(),
            string_buffers: self.string_buffers.clone(),
            type_registry: self.type_registry.clone(),
            resolved_types: self.resolved_types.clone(),
            imported_items: self.imported_items.clone(),
            warnings: self.warnings.clone(),
            unreachable_stmts: self.unreachable_stmts.clone(),
            tail_call_info,
            resolved_struct_names: self.resolved_struct_names.clone(),
            accessor_fields: self.accessor_fields.clone(),
            string_pool: HashMap::default(), // Will be populated during codegen
            function_frames,
            static_inits: self.static_inits.clone(),
            memory_config: self.memory_config.clone(),
            function_signatures: self.function_signatures.clone(),
            recursive_call_edges,
            interrupt_save_info,
            address_taken_functions: self.address_taken_functions.clone(),
            reachable_symbols,
        })
    }

    /// Allocate `size` bytes in the current function's frame, returning the offset
    /// from the (not-yet-known) frame base. `finalize_frames` later assigns each
    /// function a base and rewrites these offsets into concrete zero-page addresses.
    pub(super) fn frame_alloc(&mut self, size: u8) -> u8 {
        let offset = self.frame_cursor;
        self.frame_cursor = self.frame_cursor.saturating_add(size);
        offset
    }

    /// Allocate a hidden loop-bound slot, preferring a slot released by an
    /// earlier (sibling) loop over growing the frame. Only bound slots ever
    /// enter the free list, so reuse cannot alias user variables.
    pub(super) fn loop_bound_alloc(&mut self, size: u8) -> u8 {
        if let Some(pos) = self.loop_bound_free.iter().position(|&(_, s)| s == size) {
            let (offset, _) = self.loop_bound_free.swap_remove(pos);
            offset
        } else {
            self.frame_alloc(size)
        }
    }

    /// Release a loop-bound slot for reuse by later sibling loops.
    pub(super) fn loop_bound_release(&mut self, offset: u8, size: u8) {
        self.loop_bound_free.push((offset, size));
    }

    fn analyze_item(&mut self, item: &Spanned<Item>) -> Result<(), SemaError> {
        if let Item::Function(func) = &item.node {
            let func_name = func.name.node.clone();

            // Track current function for inline asm variable scoping
            self.current_function = Some(func_name.clone());

            // Each function gets a fresh frame; params then locals allocate upward
            // from offset 0. finalize_frames assigns the concrete base later.
            self.frame_cursor = 0;
            // Local-array data blocks restart per function too; `finalize_frames`
            // colours them against the call graph the same way frames are.
            self.array_cursor = 0;
            self.loop_bound_free.clear();

            // Check if this is an inline function
            let is_inline = func
                .attributes
                .iter()
                .any(|attr| matches!(attr, crate::ast::FnAttribute::Inline));

            self.table.enter_scope();

            // Set current return type for checking return statements
            let return_type = if let Some(ret) = &func.return_type {
                let ty = self.resolve_type(&ret.node)?;

                // Check for invalid addr usage in function return types
                if matches!(ty, Type::Primitive(PrimitiveType::Addr)) {
                    return Err(SemaError::InvalidAddrUsage {
                        context: "function return types".to_string(),
                        span: ret.span,
                    });
                }

                ty
            } else {
                Type::Void
            };
            self.current_return_type = Some(return_type);

            // Capture inline symbols for `#[inline]` functions and for auto-inline
            // candidates (the codegen inliner may expand either), tracking the
            // symbol set before body analysis to diff out this function's own.
            let capture_inline = is_inline
                || self
                    .function_metadata
                    .get(&func.name.node)
                    .is_some_and(|m| m.inline_candidate);
            let resolved_before = if capture_inline {
                Some(self.resolved_symbols.clone())
            } else {
                None
            };

            // Register parameters. Parameters occupy the bottom of the function's
            // frame as a contiguous block (offsets 0..param_bytes); locals follow.
            // finalize_frames assigns the concrete frame base. The contiguity is
            // relied upon by call.rs, which computes each argument's destination by
            // summing parameter sizes in order.
            for param in func.params.iter() {
                let name = param.name.node.clone();

                // Check for duplicate parameter names
                if self.table.defined_in_current_scope(&name) {
                    return Err(SemaError::DuplicateSymbol {
                        name: name.clone(),
                        span: param.name.span,
                        previous_span: None,
                    });
                }

                let param_type = self.resolve_type(&param.ty.node)?;

                // Check for invalid addr usage in function parameters
                if matches!(param_type, Type::Primitive(PrimitiveType::Addr)) {
                    return Err(SemaError::InvalidAddrUsage {
                        context: "function parameters".to_string(),
                        span: param.ty.span,
                    });
                }

                // Struct, enum, and array parameters are passed by reference (2-byte pointer)
                // Other types are passed by value
                let is_struct_param = matches!(param_type, Type::Named(_))
                    && self
                        .type_registry
                        .get_struct(if let Type::Named(n) = &param_type {
                            n
                        } else {
                            ""
                        })
                        .is_some();

                // Enum parameters are also passed as 2-byte pointers
                let is_enum_param = matches!(param_type, Type::Named(_))
                    && self
                        .type_registry
                        .get_enum(if let Type::Named(n) = &param_type {
                            n
                        } else {
                            ""
                        })
                        .is_some();

                // Array parameters are passed as 2-byte pointers (pass-by-reference)
                let is_array_param = matches!(param_type, Type::Array(_, _));

                let param_size = if is_struct_param || is_enum_param || is_array_param {
                    2 // Pointer size for pass-by-reference
                } else {
                    param_type.size()
                };

                // Parameter occupies a contiguous slot at the current frame offset.
                let offset = self.frame_alloc(param_size as u8);
                let location = SymbolLocation::FrameOffset(offset);

                let info = SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::Variable,
                    ty: param_type,
                    location,
                    mutable: false,
                    access_mode: None,
                    is_pub: false, // Function parameters are never public
                    containing_function: self.current_function.clone(),
                    is_param: true,
                    decl_span: Some(param.name.span),
                };
                self.table.insert(name.clone(), info.clone());
                // Add to resolved_symbols so codegen (especially inline asm) can find it
                self.resolved_symbols.insert(param.name.span, info.clone());

                // Track parameter for unused parameter detection
                self.declared_parameters.push((name, param.name.span));
            }

            // Record the contiguous parameter block size (used by recursion save/restore
            // and by call.rs for argument placement).
            let param_bytes = self.frame_cursor;
            if let Some(metadata) = self.function_metadata.get_mut(&func_name) {
                metadata.param_bytes_used = param_bytes;
            }

            // Analyze body. A failure is recorded rather than propagated, so the
            // bookkeeping below still runs: the frame size, the inline-symbol
            // capture, and — most importantly — the scope exit and
            // `current_function`/`current_return_type` reset at the end of this
            // arm. Returning early skipped all of it, leaving the analyzer in a
            // state no later function could trust. That was invisible only
            // because the first error ended the compile; it is load-bearing now
            // that the next function is still analyzed.
            if let Err(e) = self.analyze_stmt(&func.body) {
                self.record(e);
            }

            // Record this function's frame size (params + locals + any temp slots).
            // finalize_frames uses these to color frames across the call graph.
            self.frame_sizes
                .insert(func_name.clone(), self.frame_cursor);

            // For inline functions and auto-inline candidates, capture all symbols
            // added during body analysis (parameter definitions and references).
            if capture_inline && let Some(before) = resolved_before {
                // Collect all NEW symbols that were added during parameter registration and body analysis
                let mut inline_symbols = std::collections::HashMap::default();
                for (span, info) in &self.resolved_symbols {
                    if !before.contains_key(span) {
                        inline_symbols.insert(*span, info.clone());
                    }
                }

                if let Some(metadata) = self.function_metadata.get_mut(&func_name) {
                    metadata.inline_param_symbols = Some(inline_symbols);
                }
            }

            // Check for unused variables and parameters
            self.check_unused_variables();

            self.current_return_type = None;
            self.current_function = None;
            self.table.exit_scope();
        }
        Ok(())
    }

    pub(super) fn resolve_type(&mut self, ty: &TypeExpr) -> Result<Type, SemaError> {
        match ty {
            TypeExpr::Primitive(p) => Ok(Type::Primitive(*p)),
            // A `str<N>` buffer is a `str` at runtime (2-byte pointer to
            // length-prefixed data); its capacity is recorded separately, per
            // declaration, in `string_buffers`. Resolving to `Type::String`
            // means every str read op (index, `.len`, `==`, iteration, passing)
            // works on a buffer with no extra plumbing.
            TypeExpr::StringBuf { .. } => Ok(Type::String),
            TypeExpr::Named(name) => {
                // Special case: "str" maps to Type::String
                if name == "str" {
                    return Ok(Type::String);
                }

                // A name in type position is still a use: an imported struct
                // or enum named only in annotations is not an unused import.
                self.all_used_symbols.insert(name.clone());

                // Check if it's a known type (struct or enum)
                if self.type_registry.structs.contains_key(name)
                    || self.type_registry.enums.contains_key(name)
                {
                    Ok(Type::Named(name.clone()))
                } else {
                    // For now, allow unknown named types
                    // They'll be caught later if they're actually used
                    Ok(Type::Named(name.clone()))
                }
            }
            TypeExpr::Array { element, size } => {
                let element_type = self.resolve_type(&element.node)?;
                let ty = Type::Array(Box::new(element_type), *size);
                // The parser caps the element count at 65535, but bytes are
                // what the machine runs out of: `[u16; 40000]` passes the
                // count check and still cannot exist. Multiplication is done
                // by size_of, which knows named element sizes.
                let total = crate::sema::init::size_of(&ty, &self.type_registry);
                if total > 65535 {
                    return Err(SemaError::Custom {
                        message: format!(
                            "array of {} bytes exceeds the 64 KB address space",
                            total
                        ),
                        span: element.span,
                    });
                }
                Ok(ty)
            }
            TypeExpr::Slice {
                element,
                mutable: _,
            } => {
                // Slice is a fat pointer with base address and length
                let element_type = self.resolve_type(&element.node)?;
                Ok(Type::Slice(Box::new(element_type)))
            }
            TypeExpr::Pointer { pointee } => {
                // Recursing on the *type expression* is fine even for a
                // self-referential struct: an unknown name resolves to
                // `Type::Named` without a registry lookup, and `size()` stops
                // at the pointer.
                Ok(Type::Pointer(Box::new(self.resolve_type(&pointee.node)?)))
            }
            TypeExpr::Function { params, ret } => {
                let mut param_types = Vec::with_capacity(params.len());
                for p in params {
                    param_types.push(self.resolve_type(&p.node)?);
                }
                let ret_type = match ret {
                    Some(r) => self.resolve_type(&r.node)?,
                    None => Type::Void,
                };
                Ok(Type::Function(param_types, Box::new(ret_type)))
            }
        }
    }

    pub(super) fn resolve_function_type(&mut self, func: &Function) -> Result<Type, SemaError> {
        let mut param_types = Vec::with_capacity(func.params.len());
        for param in &func.params {
            param_types.push(self.resolve_type(&param.ty.node)?);
        }

        let return_type = if let Some(ret) = &func.return_type {
            self.resolve_type(&ret.node)?
        } else {
            Type::Void
        };

        Ok(Type::Function(param_types, Box::new(return_type)))
    }
}

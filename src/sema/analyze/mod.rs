//! Semantic Analysis Logic
//!
//! Traverses the AST to populate the symbol table and perform type checking.

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
use std::path::PathBuf;

pub struct SemanticAnalyzer {
    pub table: SymbolTable,
    pub errors: Vec<SemaError>,
    pub warnings: Vec<Warning>,
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
    pub(super) imported_files: HashSet<PathBuf>,
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
    /// Next free address in the BSS (RAM) section for mutable `static` globals.
    /// None until the first static is allocated. Unlike frames, BSS is never
    /// reused or colored: statics live for the whole program.
    pub(super) bss_cursor: Option<u16>,
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
            errors: Vec::with_capacity(16),
            warnings: Vec::with_capacity(16),
            current_return_type: None,
            resolved_symbols: HashMap::default(),
            function_metadata: HashMap::default(),
            folded_constants: HashMap::default(),
            loop_bound_slots: HashMap::default(),
            local_arrays: HashMap::default(),
            array_block_sizes: HashMap::default(),
            array_cursor: 0,
            resolved_types: HashMap::default(),
            type_registry: TypeRegistry::new(),
            imported_items: Vec::with_capacity(8),
            base_path: None,
            imported_files: HashSet::default(),
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
            memory_config: crate::config::MemoryConfig::load_or_default(),
            current_function: None,
            frame_cursor: 0,
            loop_bound_free: Vec::new(),
            bss_cursor: None,
            static_inits: Vec::new(),
            frame_sizes: HashMap::default(),
            function_signatures: HashMap::default(),
            call_edges: HashMap::default(),
            address_taken_functions: HashSet::default(),
            symbol_refs: HashMap::default(),
        }
    }

    pub fn with_base_path(base_path: PathBuf) -> Self {
        Self {
            table: SymbolTable::new(),
            errors: Vec::with_capacity(16),
            warnings: Vec::with_capacity(16),
            current_return_type: None,
            resolved_symbols: HashMap::default(),
            function_metadata: HashMap::default(),
            folded_constants: HashMap::default(),
            loop_bound_slots: HashMap::default(),
            local_arrays: HashMap::default(),
            array_block_sizes: HashMap::default(),
            array_cursor: 0,
            resolved_types: HashMap::default(),
            type_registry: TypeRegistry::new(),
            imported_items: Vec::with_capacity(8),
            base_path: Some(base_path),
            imported_files: HashSet::default(),
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
            memory_config: crate::config::MemoryConfig::load_or_default(),
            current_function: None,
            frame_cursor: 0,
            loop_bound_free: Vec::new(),
            bss_cursor: None,
            static_inits: Vec::new(),
            frame_sizes: HashMap::default(),
            function_signatures: HashMap::default(),
            call_edges: HashMap::default(),
            address_taken_functions: HashSet::default(),
            symbol_refs: HashMap::default(),
        }
    }

    /// Get the standard library path
    /// Checks WRAITH_STD_PATH environment variable, falls back to ./std
    pub(super) fn get_std_lib_path() -> PathBuf {
        std::env::var("WRAITH_STD_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("std"))
    }

    /// Compute the size of a type, looking up named types in the registry
    pub(super) fn type_size(&self, ty: &Type) -> usize {
        match ty {
            Type::Primitive(prim) => prim.size_bytes(),
            Type::Array(element_ty, len) => self.type_size(element_ty) * len,
            Type::Slice(_) => 4, // Fat pointer: 2 bytes base address + 2 bytes length
            Type::Pointer(_) => 2, // 16-bit address; never recurse into the pointee
            Type::String => 2,   // String is represented as a pointer
            Type::Function(_, _) => 2, // Function pointer is 16-bit
            Type::Void => 0,
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
        // First pass: Register all global items (functions, statics, structs)
        for item in &source.items {
            self.register_item(item)?;
        }

        // Second pass: Analyze function bodies
        for item in &source.items {
            self.analyze_item(item)?;
        }

        if !self.errors.is_empty() {
            return Err(self.errors[0].clone());
        }

        // Check for unused imports after all analysis is complete. Unused
        // *functions* are reported from `analyze` instead: whether one is dead
        // is a whole-program question, and only the root sees the whole
        // program.
        self.check_unused_imports();

        // Analyze tail calls after all other analysis is complete
        Ok(self.analyze_tail_calls(source))
    }

    pub fn analyze(&mut self, source: &SourceFile) -> Result<ProgramInfo, SemaError> {
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
        let finalized = self.finalize_frames()?;
        let function_frames = finalized.frames;
        let recursive_call_edges = finalized.recursive_call_edges;
        let interrupt_save_info = finalized.interrupt_save_info;

        Ok(ProgramInfo {
            table: self.table.clone(),
            resolved_symbols: self.resolved_symbols.clone(),
            function_metadata: self.function_metadata.clone(),
            folded_constants: self.folded_constants.clone(),
            loop_bound_slots: self.loop_bound_slots.clone(),
            local_arrays: self.local_arrays.clone(),
            type_registry: self.type_registry.clone(),
            resolved_types: self.resolved_types.clone(),
            imported_items: self.imported_items.clone(),
            warnings: self.warnings.clone(),
            unreachable_stmts: self.unreachable_stmts.clone(),
            tail_call_info,
            resolved_struct_names: self.resolved_struct_names.clone(),
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

            // For inline functions, track symbols before body analysis
            let resolved_before = if is_inline {
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

            // Analyze body
            self.analyze_stmt(&func.body)?;

            // Record this function's frame size (params + locals + any temp slots).
            // finalize_frames uses these to color frames across the call graph.
            self.frame_sizes
                .insert(func_name.clone(), self.frame_cursor);

            // For inline functions, capture all symbols that were added during body analysis
            // This includes both parameter definitions and all references to them
            if is_inline && let Some(before) = resolved_before {
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

    pub(super) fn resolve_type(&self, ty: &TypeExpr) -> Result<Type, SemaError> {
        match ty {
            TypeExpr::Primitive(p) => Ok(Type::Primitive(*p)),
            TypeExpr::Named(name) => {
                // Special case: "str" maps to Type::String
                if name == "str" {
                    return Ok(Type::String);
                }

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
                Ok(Type::Array(Box::new(element_type), *size))
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

    pub(super) fn resolve_function_type(&self, func: &Function) -> Result<Type, SemaError> {
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

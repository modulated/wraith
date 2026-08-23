//! Flattening a constant initializer into the bytes it denotes.
//!
//! Two places need this and used to do it separately: a mutable `static` needs
//! a startup image for the reset handler to write into RAM, and a `const` array
//! needs ROM data. Both were shallow — they understood an integer, an array of
//! integers, and a struct of integers, but nothing nested. An array *of*
//! structs matched none of those cases, so the `static` path fell through to
//! its zero-fill fallback and silently produced a table of zeros, while the
//! `const` path reported "Array elements must be constants".
//!
//! The fix is to make the walk recursive and drive it from the declared *type*
//! rather than from the shape of the expression. Each field and element is
//! padded out to its own width before the next begins, so a short initializer,
//! a missing struct field and a nested aggregate all lay out correctly.
//!
//! An aggregate whose parts cannot be resolved is now an error rather than
//! zeros. Zero remains the fallback only for a scalar, where it is the meaning
//! of BSS and the common `= 0` case.

use crate::ast::{Expr, Literal, PrimitiveType, Span, Spanned};
use crate::sema::InitByte;
use crate::sema::type_defs::TypeRegistry;
use crate::sema::types::Type;

/// Why an initializer could not be turned into bytes.
pub struct InitError {
    pub message: String,
    pub span: Span,
    /// Whether this is a definite mistake rather than merely "no compile-time
    /// value here". A forward reference or a malformed aggregate is always an
    /// error; a scalar the compiler cannot fold is only an error where a value
    /// was required, which is the caller's call to make.
    pub fatal: bool,
}

impl InitError {
    fn fatal(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
            fatal: true,
        }
    }

    /// Promote a nested failure. "No compile-time value" is tolerable for a
    /// whole scalar `static`, whose storage is undefined RAM anyway, but not
    /// for one element or field of an aggregate: something was clearly meant to
    /// go there, and zeros would be indistinguishable from a table never
    /// written.
    fn required(mut self) -> Self {
        self.fatal = true;
        self
    }
}

/// The size of a type in bytes.
///
/// The one place this is written down for aggregates: a struct or enum knows
/// its own total, an array multiplies, and a pointer is 2 bytes *without*
/// recursing into the pointee — which is what lets `struct Node { next: &Node }`
/// terminate.
pub fn size_of(ty: &Type, registry: &TypeRegistry) -> usize {
    match ty {
        Type::Array(element, len) => size_of(element, registry) * len,
        Type::Named(name) => registry
            .structs
            .get(name)
            .map(|s| s.total_size)
            .or_else(|| registry.enums.get(name).map(|e| e.total_size))
            .unwrap_or(0),
        other => other.size(),
    }
}

/// What the flattener needs from its caller.
///
/// The two callers resolve names differently — semantic analysis has the
/// constant environment, codegen has the folded-constant map keyed by span — so
/// that lookup is supplied rather than assumed.
pub trait InitContext {
    fn registry(&self) -> &TypeRegistry;

    /// The integer value of a constant expression, if it has one.
    fn integer(&self, expr: &Spanned<Expr>) -> Option<i64>;

    /// The function this expression names, if it names one. Its address is only
    /// known to the assembler, so it becomes a label reference.
    fn function_name(&self, expr: &Spanned<Expr>) -> Option<String>;

    /// The address `&operand` denotes.
    fn address_of(&self, operand: &Spanned<Expr>) -> Result<u16, InitError>;

    /// The folded entries of the generated table at `span`, if there is one.
    ///
    /// Supplied rather than recomputed: sema folds each table once, and two
    /// sites folding it separately is two chances to disagree.
    fn generated_table(&self, span: Span) -> Option<&[i64]>;
}

/// Flatten `expr` into exactly `size_of(ty)` bytes appended to `out`.
pub fn flatten(
    out: &mut Vec<InitByte>,
    expr: &Spanned<Expr>,
    ty: &Type,
    ctx: &dyn InitContext,
) -> Result<(), InitError> {
    let base = out.len();
    let width = size_of(ty, ctx.registry());
    flatten_inner(out, expr, ty, ctx)?;
    pad_to(out, base + width);
    Ok(())
}

fn pad_to(out: &mut Vec<InitByte>, target: usize) {
    while out.len() < target {
        out.push(InitByte::Byte(0));
    }
}

/// Whether `value` is representable in `ty`. Only the plain integer types
/// are checked: pointers, `&`-addresses and function labels have their own
/// arms above, and BCD values carry their own validation in const_eval.
fn fits_in_type(value: i64, ty: &Type) -> bool {
    match ty {
        Type::Primitive(PrimitiveType::U8) => (0..=255).contains(&value),
        Type::Primitive(PrimitiveType::I8) => (-128..=127).contains(&value),
        Type::Primitive(PrimitiveType::U16) => (0..=65535).contains(&value),
        Type::Primitive(PrimitiveType::I16) => (-32768..=32767).contains(&value),
        Type::Primitive(PrimitiveType::Bool) => (0..=1).contains(&value),
        _ => true,
    }
}

fn push_int(out: &mut Vec<InitByte>, value: i64, width: usize) {
    for i in 0..width {
        out.push(InitByte::Byte(((value >> (i * 8)) & 0xFF) as u8));
    }
}

fn flatten_inner(
    out: &mut Vec<InitByte>,
    expr: &Spanned<Expr>,
    ty: &Type,
    ctx: &dyn InitContext,
) -> Result<(), InitError> {
    // Parentheses carry no meaning here.
    if let Expr::Paren(inner) = &expr.node {
        return flatten_inner(out, inner, ty, ctx);
    }

    match ty {
        Type::Array(element, len) => flatten_array(out, expr, element, *len, ctx),
        Type::Named(name) if ctx.registry().structs.contains_key(name) => {
            flatten_struct(out, expr, name, ctx)
        }
        Type::Named(name) if ctx.registry().enums.contains_key(name) => {
            flatten_enum(out, expr, name, ctx)
        }
        _ => flatten_scalar(out, expr, ty, ctx),
    }
}

/// An enum field: its tag byte, then whatever the variant carries.
///
/// An enum is stored inline like a struct — the tag and the payload, not a
/// pointer to them — so the bytes are the same shape as a struct's. What made
/// this the missing case is that a variant is spelled `C::B` rather than as a
/// number, and nothing folded that to its discriminant.
fn flatten_enum(
    out: &mut Vec<InitByte>,
    expr: &Spanned<Expr>,
    name: &str,
    ctx: &dyn InitContext,
) -> Result<(), InitError> {
    use crate::ast::VariantData;

    let Expr::EnumVariant {
        enum_name,
        variant,
        data,
    } = &expr.node
    else {
        return Err(InitError::fatal(
            format!("`{name}` is an enum, so it needs one of its variants here"),
            expr.span,
        ));
    };
    if enum_name.node != name {
        return Err(InitError::fatal(
            format!(
                "`{}` is not a variant of `{name}`",
                format_args!("{}::{}", enum_name.node, variant.node)
            ),
            expr.span,
        ));
    }
    let def = ctx
        .registry()
        .enums
        .get(name)
        .expect("checked by the caller");
    let info = def.get_variant(&variant.node).ok_or_else(|| {
        InitError::fatal(
            format!("`{name}` has no variant `{}`", variant.node),
            expr.span,
        )
    })?;
    let base = out.len();
    out.push(InitByte::Byte(info.tag));

    // The payload follows the tag, laid out by the *declaration's* field types
    // rather than by the initializer's shape — the same rule as a struct.
    match (&info.data, data) {
        (crate::sema::type_defs::VariantData::Unit, VariantData::Unit) => Ok(()),
        (crate::sema::type_defs::VariantData::Tuple(types), VariantData::Tuple(values)) => {
            if values.len() != types.len() {
                return Err(InitError::fatal(
                    format!(
                        "`{}::{}` carries {} value(s) but {} were given",
                        name,
                        variant.node,
                        types.len(),
                        values.len()
                    ),
                    expr.span,
                ));
            }
            let mut at = base + 1;
            for (t, v) in types.iter().zip(values) {
                pad_to(out, at);
                flatten_inner(out, v, t, ctx).map_err(InitError::required)?;
                at += size_of(t, ctx.registry()).max(1);
            }
            Ok(())
        }
        (crate::sema::type_defs::VariantData::Struct(fields), VariantData::Struct(inits)) => {
            for f in fields {
                let Some(init) = inits.iter().find(|x| x.name.node == f.name) else {
                    continue;
                };
                pad_to(out, base + 1 + f.offset);
                flatten_inner(out, &init.value, &f.ty, ctx).map_err(InitError::required)?;
            }
            Ok(())
        }
        _ => Err(InitError::fatal(
            format!("`{}::{}` is not written that way", name, variant.node),
            expr.span,
        )),
    }
}

fn flatten_array(
    out: &mut Vec<InitByte>,
    expr: &Spanned<Expr>,
    element: &Type,
    len: usize,
    ctx: &dyn InitContext,
) -> Result<(), InitError> {
    let elem_size = size_of(element, ctx.registry()).max(1);
    let base = out.len();

    match &expr.node {
        Expr::Literal(Literal::Array(elems)) => {
            if elems.len() > len {
                return Err(InitError::fatal(
                    format!(
                        "this initializer has {} elements but the type holds {}",
                        elems.len(),
                        len
                    ),
                    expr.span,
                ));
            }
            for (i, e) in elems.iter().enumerate() {
                pad_to(out, base + i * elem_size);
                flatten_inner(out, e, element, ctx).map_err(InitError::required)?;
            }
            Ok(())
        }
        Expr::Literal(Literal::ArrayFill { value, count }) => {
            if *count > len {
                return Err(InitError::fatal(
                    format!(
                        "this fill initializer repeats {} times but the array holds {}",
                        count, len
                    ),
                    expr.span,
                ));
            }
            for i in 0..*count {
                pad_to(out, base + i * elem_size);
                flatten_inner(out, value, element, ctx).map_err(InitError::required)?;
            }
            Ok(())
        }
        // A generated table: one entry per index, already folded by sema.
        Expr::Literal(Literal::ArrayGen { .. }) => {
            let Some(values) = ctx.generated_table(expr.span) else {
                return Err(InitError::fatal(
                    "this generated table was never folded, which is a compiler bug".to_string(),
                    expr.span,
                ));
            };
            if values.len() != len {
                return Err(InitError::fatal(
                    format!(
                        "this generated table holds {} entries but the type holds {len}",
                        values.len()
                    ),
                    expr.span,
                ));
            }
            for (i, v) in values.iter().enumerate() {
                pad_to(out, base + i * elem_size);
                push_int(out, *v, elem_size);
            }
            Ok(())
        }
        // A string initializing a byte array lays down its characters. The
        // length prefix belongs to the `str` representation, not to `[u8; N]`.
        Expr::Literal(Literal::String(s)) if elem_size == 1 => {
            for b in s.as_bytes().iter().take(len) {
                out.push(InitByte::Byte(*b));
            }
            Ok(())
        }
        _ => Err(InitError::fatal(
            "an array needs a list or a fill initializer",
            expr.span,
        )),
    }
}

fn flatten_struct(
    out: &mut Vec<InitByte>,
    expr: &Spanned<Expr>,
    struct_name: &str,
    ctx: &dyn InitContext,
) -> Result<(), InitError> {
    let fields = match &expr.node {
        Expr::StructInit { fields, .. } | Expr::AnonStructInit { fields } => fields,
        _ => {
            return Err(InitError::fatal(
                format!("'{}' needs a struct literal initializer", struct_name),
                expr.span,
            ));
        }
    };

    let def = ctx
        .registry()
        .structs
        .get(struct_name)
        .expect("checked by the caller");
    let base = out.len();

    // A name the definition doesn't have would be skipped silently by the
    // walk below.
    for init in fields {
        if !def.fields.iter().any(|f| f.name == init.name.node) {
            return Err(InitError::fatal(
                format!("struct '{}' has no field '{}'", struct_name, init.name.node),
                init.name.span,
            ));
        }
    }

    // Walk the *definition*, not the initializer, so fields land at their
    // declared offsets whatever order they were written in. An omitted field is
    // left as the zeros `pad_to` supplies.
    for f in &def.fields {
        let Some(init) = fields.iter().find(|x| x.name.node == f.name) else {
            continue;
        };
        pad_to(out, base + f.offset);
        flatten_inner(out, &init.value, &f.ty, ctx).map_err(InitError::required)?;
    }
    Ok(())
}

fn flatten_scalar(
    out: &mut Vec<InitByte>,
    expr: &Spanned<Expr>,
    ty: &Type,
    ctx: &dyn InitContext,
) -> Result<(), InitError> {
    let width = size_of(ty, ctx.registry()).max(1);

    // A function name contributes its address as two label references; only the
    // assembler knows where it landed. This is what a device vtable is made of.
    if let Some(f) = ctx.function_name(expr) {
        out.push(InitByte::FnLow(f.clone()));
        out.push(InitByte::FnHigh(f));
        return Ok(());
    }

    // `&OTHER` — a static's address is already fixed by the time this runs.
    if let Expr::Unary {
        op: crate::ast::UnaryOp::AddrOf,
        operand,
    } = &expr.node
    {
        let addr = ctx.address_of(operand)?;
        push_int(out, addr as i64, 2);
        return Ok(());
    }

    // A `str` is a pointer at the literal's data, which is the same shape as a
    // function field: two bytes the assembler fills in. What differs is only
    // *who* names the label — the string collector does, at codegen — so the
    // content travels and the label is resolved there.
    if let Expr::Literal(Literal::String(text)) = &expr.node
        && matches!(ty, Type::String)
    {
        out.push(InitByte::StrLow(text.clone()));
        out.push(InitByte::StrHigh(text.clone()));
        return Ok(());
    }

    if let Expr::Literal(Literal::Bool(b)) = &expr.node {
        // Matches the implicit-conversion rules: a bool can stand in for a
        // u8, but not for a wider or signed type.
        if !matches!(
            ty,
            Type::Primitive(PrimitiveType::Bool) | Type::Primitive(PrimitiveType::U8)
        ) {
            return Err(InitError::fatal(
                format!("a boolean literal cannot initialize {}", ty.display_name()),
                expr.span,
            ));
        }
        out.push(InitByte::Byte(*b as u8));
        return Ok(());
    }

    if let Some(v) = ctx.integer(expr) {
        if !fits_in_type(v, ty) {
            return Err(InitError::fatal(
                format!("{} does not fit in {}", v, ty.display_name()),
                expr.span,
            ));
        }
        push_int(out, v, width);
        return Ok(());
    }

    // Not resolvable. Not necessarily wrong, either: at the top level of a
    // scalar `static` it means "whatever RAM holds", which is what BSS is. Mark
    // it non-fatal and let `flatten_top` decide.
    Err(InitError {
        message: "this initializer is not a compile-time constant".to_string(),
        span: expr.span,
        fatal: false,
    })
}

/// Flatten a whole declaration, optionally tolerating an initializer with no
/// compile-time value.
///
/// A mutable `static` lives in RAM, which is undefined at power-on, so a scalar
/// the compiler cannot evaluate simply starts at zero — that is the meaning of
/// BSS and the reason `= 0` needs no special case. A `const` in ROM has no such
/// excuse and passes `false`.
///
/// Only the *non-fatal* case is tolerated. A forward reference, `&CONST`, or a
/// malformed aggregate is a mistake with a diagnosis attached, and swallowing
/// it would put the reader back where they started: a table of zeros and
/// nothing said about it.
pub fn flatten_top(
    expr: &Spanned<Expr>,
    ty: &Type,
    ctx: &dyn InitContext,
    tolerate_unknown: bool,
    soa: Option<&crate::sema::SoaLayout>,
) -> Result<Vec<InitByte>, InitError> {
    let size = size_of(ty, ctx.registry());
    let mut out = Vec::with_capacity(size);
    match flatten(&mut out, expr, ty, ctx) {
        Ok(()) => Ok(transpose(out, soa, ctx)),
        Err(e) if tolerate_unknown && !e.fatal => Ok(vec![InitByte::Byte(0); size]),
        Err(e) => Err(e),
    }
}

/// Rearrange a flattened array of structs into one column per field.
///
/// The initializer is written as records and flattened as records, then turned
/// on its side here — once, at the only place a whole declaration's image is
/// produced. Doing it this way means every initializer form that already works
/// (a literal, a fill, a nested enum, a `&OTHER`) keeps working without knowing
/// the layout exists; the alternative was a second flattener that had to agree
/// with the first about every one of those forms.
fn transpose(
    bytes: Vec<InitByte>,
    soa: Option<&crate::sema::SoaLayout>,
    ctx: &dyn InitContext,
) -> Vec<InitByte> {
    let Some(layout) = soa else { return bytes };
    let Some(sdef) = ctx.registry().get_struct(&layout.elem).cloned() else {
        return bytes;
    };
    let elem_size = sdef.total_size;

    let mut out = vec![InitByte::Byte(0); bytes.len()];
    for i in 0..layout.len {
        for f in &sdef.fields {
            let width = size_of(&f.ty, ctx.registry()).max(1);
            let from = i * elem_size + f.offset;
            let to = layout.column(f.offset) + i * width;
            for k in 0..width {
                if from + k < bytes.len() && to + k < out.len() {
                    out[to + k] = bytes[from + k].clone();
                }
            }
        }
    }
    out
}

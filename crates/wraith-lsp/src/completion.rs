//! Completion candidates.
//!
//! Two contexts:
//!
//! - **Member access** (`receiver.<partial>`): the fields of `receiver`'s
//!   struct type. This is the case that needs the semantic model — we resolve
//!   the receiver's type from the last good analysis and list its fields.
//! - **Everything else**: keywords, the primitive types, and the program's
//!   top-level declarations (functions, structs, enums, statics). These come
//!   straight from the parsed tree and are always correct.
//!
//! Locals and parameters are deliberately *not* offered in the general case
//! yet: that needs a scope-at-position query the batch compiler does not expose,
//! and offering every local from every function would be misleading. That is
//! the next slice, not a silent gap.

use lsp_types::{CompletionItem, CompletionItemKind};
use wraith::ast::Item;
use wraith::sema::types::Type;

use crate::document::Doc;

const KEYWORDS: &[&str] = &[
    "fn", "let", "if", "else", "loop", "while", "for", "in", "match", "return", "break",
    "continue", "struct", "enum", "static", "const", "pub", "import", "from", "as", "true",
    "false", "asm",
];

const TYPES: &[&str] = &["u8", "i8", "u16", "i16", "b8", "b16", "bool", "str", "addr"];

pub fn complete(doc: &Doc, offset: usize) -> Vec<CompletionItem> {
    let before = &doc.text[..offset.min(doc.text.len())];
    if let Some(receiver) = member_receiver(before) {
        return field_completions(doc, &receiver);
    }
    general_completions(doc)
}

/// If the text immediately before the cursor is `ident.` (optionally followed by
/// a partial identifier being typed), return the receiver identifier.
fn member_receiver(before: &str) -> Option<String> {
    let b = before.as_bytes();
    let mut i = b.len();
    // Skip the partial identifier under the cursor.
    while i > 0 && is_ident_byte(b[i - 1]) {
        i -= 1;
    }
    // A dot must sit right before it.
    if i == 0 || b[i - 1] != b'.' {
        return None;
    }
    i -= 1;
    // Capture the receiver identifier before the dot.
    let end = i;
    while i > 0 && is_ident_byte(b[i - 1]) {
        i -= 1;
    }
    if i == end {
        return None;
    }
    Some(before[i..end].to_string())
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Fields of the struct that `receiver` has, resolved from the last good
/// analysis. Looks through one level of pointer, matching `p.field`.
fn field_completions(doc: &Doc, receiver: &str) -> Vec<CompletionItem> {
    let Some(good) = doc.good.as_ref() else {
        return Vec::new();
    };

    // Find any symbol with this name and take its type. Flow-insensitive and
    // shadowing-blind, which is acceptable for completion.
    let ty = good
        .info
        .resolved_symbols
        .values()
        .find(|s| s.name == receiver)
        .map(|s| &s.ty);

    let struct_name = match ty {
        Some(Type::Named(n)) => Some(n.as_str()),
        Some(Type::Pointer(inner)) => match &**inner {
            Type::Named(n) => Some(n.as_str()),
            _ => None,
        },
        _ => None,
    };

    let Some(def) = struct_name.and_then(|n| good.info.type_registry.get_struct(n)) else {
        return Vec::new();
    };

    def.fields
        .iter()
        .map(|f| CompletionItem {
            label: f.name.clone(),
            kind: Some(CompletionItemKind::FIELD),
            detail: Some(f.ty.display_name()),
            ..Default::default()
        })
        .collect()
}

fn general_completions(doc: &Doc) -> Vec<CompletionItem> {
    let mut items = Vec::new();

    for kw in KEYWORDS {
        items.push(CompletionItem {
            label: (*kw).to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            ..Default::default()
        });
    }
    for ty in TYPES {
        items.push(CompletionItem {
            label: (*ty).to_string(),
            kind: Some(CompletionItemKind::TYPE_PARAMETER),
            ..Default::default()
        });
    }

    // Top-level declarations from the last good parse.
    if let Some(good) = doc.good.as_ref() {
        for item in &good.ast.items {
            let (label, kind, detail) = match &item.node {
                Item::Function(f) => (
                    f.name.node.clone(),
                    CompletionItemKind::FUNCTION,
                    signature(f),
                ),
                Item::Struct(s) => (
                    s.name.node.clone(),
                    CompletionItemKind::STRUCT,
                    "struct".into(),
                ),
                Item::Enum(e) => (e.name.node.clone(), CompletionItemKind::ENUM, "enum".into()),
                Item::Static(s) => (
                    s.name.node.clone(),
                    if s.mutable {
                        CompletionItemKind::VARIABLE
                    } else {
                        CompletionItemKind::CONSTANT
                    },
                    if s.mutable { "static" } else { "const" }.into(),
                ),
                Item::Address(a) => (
                    a.name.node.clone(),
                    CompletionItemKind::CONSTANT,
                    "addr".into(),
                ),
                Item::Import(_) => continue,
            };
            items.push(CompletionItem {
                label,
                kind: Some(kind),
                detail: Some(detail),
                ..Default::default()
            });
        }
    }

    items
}

fn signature(f: &wraith::ast::Function) -> String {
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name.node, type_expr_name(&p.ty.node)))
        .collect();
    let ret = f
        .return_type
        .as_ref()
        .map(|t| format!(" -> {}", type_expr_name(&t.node)))
        .unwrap_or_default();
    format!("fn {}({}){}", f.name.node, params.join(", "), ret)
}

/// A short display name for a syntactic type. Good enough for a completion
/// detail line; not a full resolver.
fn type_expr_name(t: &wraith::ast::TypeExpr) -> String {
    use wraith::ast::TypeExpr;
    match t {
        TypeExpr::Primitive(p) => format!("{:?}", p).to_lowercase(),
        TypeExpr::Named(n) => n.clone(),
        TypeExpr::Pointer { pointee } => format!("&{}", type_expr_name(&pointee.node)),
        TypeExpr::Array { element, size } => {
            format!("[{}; {}]", type_expr_name(&element.node), size)
        }
        TypeExpr::Slice { element, .. } => format!("&[{}]", type_expr_name(&element.node)),
        _ => "?".to_string(),
    }
}

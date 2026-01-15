//! Integration tests for the Wraith AST

use wraith::ast::*;

#[test]
fn test_primitive_types() {
    assert_eq!(PrimitiveType::U8.size_bytes(), 1);
    assert_eq!(PrimitiveType::I8.size_bytes(), 1);
    assert_eq!(PrimitiveType::Bool.size_bytes(), 1);
    assert_eq!(PrimitiveType::U16.size_bytes(), 2);
    assert_eq!(PrimitiveType::I16.size_bytes(), 2);
}

#[test]
fn test_span_merge() {
    let span1 = Span::new(0, 10);
    let span2 = Span::new(5, 20);
    let merged = span1.merge(span2);
    assert_eq!(merged, Span::new(0, 20));
}

#[test]
fn test_expression_construction() {
    // Test integer literal
    let int_expr = Expr::int(42);
    assert!(matches!(int_expr, Expr::Literal(Literal::Integer(42))));

    // Test variable reference
    let var_expr = Expr::var("counter");
    assert!(matches!(var_expr, Expr::Variable(name) if name == "counter"));

    // Test binary expression
    let left = Spanned::dummy(Expr::int(1));
    let right = Spanned::dummy(Expr::int(2));
    let binary = Expr::binary(left, BinaryOp::Add, right);
    assert!(matches!(
        binary,
        Expr::Binary {
            op: BinaryOp::Add,
            ..
        }
    ));
}

#[test]
fn test_type_expressions() {
    // Array type
    let element = Spanned::dummy(TypeExpr::primitive(PrimitiveType::U16));
    let array_type = TypeExpr::array(element.clone(), 10);
    assert!(matches!(array_type, TypeExpr::Array { size: 10, .. }));

    // Slice type
    let slice_type = TypeExpr::slice(element, true);
    assert!(matches!(slice_type, TypeExpr::Slice { mutable: true, .. }));
}

#[test]
fn test_function_definition() {
    let func = Function {
        name: Spanned::dummy("add".to_string()),
        params: vec![
            FnParam {
                name: Spanned::dummy("a".to_string()),
                ty: Spanned::dummy(TypeExpr::primitive(PrimitiveType::U8)),
            },
            FnParam {
                name: Spanned::dummy("b".to_string()),
                ty: Spanned::dummy(TypeExpr::primitive(PrimitiveType::U8)),
            },
        ],
        return_type: Some(Spanned::dummy(TypeExpr::primitive(PrimitiveType::U8))),
        body: Spanned::dummy(Stmt::Block(vec![])),
        attributes: vec![],
        is_pub: false,
    };

    assert_eq!(func.name.node, "add");
    assert_eq!(func.params.len(), 2);
}

#[test]
fn test_struct_definition() {
    let point = Struct {
        name: Spanned::dummy("Point".to_string()),
        fields: vec![
            StructField {
                name: Spanned::dummy("x".to_string()),
                ty: Spanned::dummy(TypeExpr::primitive(PrimitiveType::U8)),
            },
            StructField {
                name: Spanned::dummy("y".to_string()),
                ty: Spanned::dummy(TypeExpr::primitive(PrimitiveType::U8)),
            },
        ],
        is_pub: false,
        attributes: vec![],
    };

    assert_eq!(point.fields.len(), 2);
}

#[test]
fn test_enum_with_variants() {
    let message = Enum {
        name: Spanned::dummy("Message".to_string()),
        variants: vec![
            EnumVariant::Unit {
                name: Spanned::dummy("Quit".to_string()),
                value: None,
            },
            EnumVariant::Struct {
                name: Spanned::dummy("Move".to_string()),
                fields: vec![
                    StructField {
                        name: Spanned::dummy("x".to_string()),
                        ty: Spanned::dummy(TypeExpr::primitive(PrimitiveType::U8)),
                    },
                    StructField {
                        name: Spanned::dummy("y".to_string()),
                        ty: Spanned::dummy(TypeExpr::primitive(PrimitiveType::U8)),
                    },
                ],
            },
            EnumVariant::Tuple {
                name: Spanned::dummy("Write".to_string()),
                fields: vec![Spanned::dummy(TypeExpr::primitive(PrimitiveType::U8))],
            },
        ],
        is_pub: false,
    };

    assert_eq!(message.variants.len(), 3);
    assert_eq!(message.variants[0].name(), "Quit");
    assert_eq!(message.variants[1].name(), "Move");
    assert_eq!(message.variants[2].name(), "Write");
}

#[test]
fn test_source_file() {
    let file = SourceFile::with_items(vec![
        Spanned::dummy(Item::Struct(Struct {
            is_pub: false,
            name: Spanned::dummy("Point".to_string()),
            fields: vec![],
            attributes: vec![],
        })),
        Spanned::dummy(Item::Function(Box::new(Function {
            name: Spanned::dummy("main".to_string()),
            is_pub: false,
            params: vec![],
            return_type: Some(Spanned::dummy(TypeExpr::primitive(PrimitiveType::U8))),
            body: Spanned::dummy(Stmt::Block(vec![])),
            attributes: vec![],
        }))),
    ]);

    assert_eq!(file.items.len(), 2);
}

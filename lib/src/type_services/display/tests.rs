use super::*;
use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId, TypeVarId};
use crate::types::{AssociatedTypeKey, GenericParamId};

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

#[test]
fn type_display_service_preserves_existing_output() {
    let generic = GenericParamId {
        owner: def_id(4),
        index: 1,
    };
    let projection = Type::Projection {
        ty: Box::new(Type::Generic(generic)),
        trait_id: def_id(5),
        assoc_type: AssociatedTypeKey {
            owner: def_id(5),
            assoc_type_id: AssocTypeId(2),
        },
        trait_args: vec![Type::I64, Type::Bool],
    };

    let cases = vec![
        (Type::I8, "I8"),
        (Type::U64, "U64"),
        (Type::F32, "F32"),
        (Type::Bool, "Bool"),
        (Type::Str, "Str"),
        (Type::Char, "Char"),
        (Type::Unit, "()"),
        (Type::Never, "!"),
        (Type::Slice(Box::new(Type::I64)), "[I64]"),
        (Type::Array(Box::new(Type::Bool), 4), "[Bool; 4]"),
        (Type::Tuple(vec![Type::I64, Type::Bool]), "(I64, Bool)"),
        (
            Type::function(vec![Type::I64, Type::Bool], Type::Unit),
            "I64 -> Bool -> ()",
        ),
        (
            Type::Struct {
                id: def_id(1),
                args: vec![Type::I64],
            },
            "struct#0::1<I64>",
        ),
        (
            Type::Enum {
                id: def_id(2),
                args: vec![Type::Bool],
            },
            "enum#0::2<Bool>",
        ),
        (
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Str),
            },
            "&Str",
        ),
        (
            Type::Reference {
                mutable: true,
                inner: Box::new(Type::I64),
            },
            "&mut I64",
        ),
        (Type::Pointer(Box::new(Type::I64)), "*I64"),
        (Type::TypeVar(TypeVarId(7)), "?T7"),
        (Type::Generic(generic), "generic#0::4.1"),
        (
            projection,
            "<generic#0::4.1 as trait#0::5<I64, Bool>>::assoc#0::5.2",
        ),
        (Type::Error, "<error>"),
    ];

    for (ty, expected) in cases {
        assert_eq!(display_type(&ty).to_string(), expected);
        assert_eq!(ty.to_string(), expected);
    }
}

#[test]
fn user_type_display_uses_resolved_rock_names() {
    let owner = def_id(1);
    let result = def_id(2);
    let trait_id = def_id(5);
    let generic = GenericParamId { owner, index: 0 };
    let associated = AssociatedTypeKey {
        owner: trait_id,
        assoc_type_id: AssocTypeId(2),
    };
    let mut resolver = ResolverTables::default();
    resolver
        .item_names_by_id
        .insert(owner, "PtrBox".to_string());
    resolver
        .item_names_by_id
        .insert(result, "Result".to_string());
    resolver
        .item_names_by_id
        .insert(trait_id, "Iterator".to_string());
    let mut context = TypeDisplayContext::from_resolver(&resolver);
    context.insert_generic_name(generic, "T");
    context.insert_associated_name(associated, "Item");

    let ptr_box = Type::Struct {
        id: owner,
        args: vec![Type::Slice(Box::new(Type::I64))],
    };
    assert_eq!(
        display_type_with_context(&ptr_box, &context).to_string(),
        "PtrBox [I64]"
    );
    let nested = Type::Enum {
        id: result,
        args: vec![Type::Generic(generic), ptr_box],
    };
    assert_eq!(
        display_type_with_context(&nested, &context).to_string(),
        "Result T, PtrBox [I64]"
    );
    let projection = Type::Projection {
        ty: Box::new(Type::Generic(generic)),
        trait_id,
        assoc_type: associated,
        trait_args: Vec::new(),
    };
    assert_eq!(
        display_type_with_context(&projection, &context).to_string(),
        "<T as Iterator>::Item"
    );
}

#[test]
fn user_type_display_never_exposes_unknown_semantic_ids() {
    let context = TypeDisplayContext::default();
    let unknown = Type::Struct {
        id: def_id(99),
        args: vec![Type::TypeVar(TypeVarId(7))],
    };
    let rendered = display_type_with_context(&unknown, &context).to_string();
    assert_eq!(rendered, "<unknown type> _");
    assert!(!rendered.contains("struct#"));
    assert!(!rendered.contains("?T"));
}

use std::collections::BTreeMap;

use super::*;

#[test]
fn h3_layout_errors_propagate_for_symbolic_array_lengths() {
    use crate::types::ArrayLen;
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "h3_layout");
    for len in [ArrayLen::Error, ArrayLen::Var(crate::ids::TypeVarId(7)), ArrayLen::Bound { depth: 0, index: 0 }, ArrayLen::Param(crate::types::GenericParamId { owner: DefId::new(CrateId(0), LocalDefId(1)), index: 0 })] {
        let array = Type::Array(Box::new(Type::I64), len);
        let mut types = crate::type_context::TypeContext::new();
        let id = types.intern_canonical_type(&array);
        codegen.set_type_context(types);
        for result in [
            codegen.llvm_type(&array),
            codegen.llvm_type_id(id),
            codegen.llvm_type(&Type::Tuple(vec![array.clone()])),
        ] {
            assert!(result.unwrap_err().message.contains("not concrete"));
        }
        assert!(codegen.callable_code_type(&[array.clone()], &Type::Unit).is_err());
        assert!(codegen.callable_code_type(&[], &array).is_err());
        assert!(codegen.llvm_param_type(&crate::mir::MirParamAbi { semantic_ty: id, pass_mode: crate::mir::MirPassMode::Direct }).is_err());
        let owner = DefId::new(CrateId(0), LocalDefId(2));
        codegen.nominal_layouts.register_struct(owner, vec![("items".to_string(), id)], Vec::new());
        assert!(codegen.llvm_type(&Type::Struct { id: owner, args: Vec::new() }).is_err());
        codegen.nominal_layouts.register_enum(owner, vec![crate::codegen::CodegenEnumVariantLayout {
            fields: crate::codegen::CodegenEnumVariantFields::Positional(vec![id]),
        }], Vec::new());
        assert!(codegen.enum_layout_types_by_id(owner, &[]).is_err());
    }
    for len in [0, 3] {
        assert_eq!(codegen.llvm_type(&Type::Array(Box::new(Type::I64), ArrayLen::Value(len))).unwrap().into_array_type().len(), len as u32);
    }
    assert!(codegen.llvm_type(&Type::Array(Box::new(Type::I64), ArrayLen::Value(u64::MAX))).is_err());
}

use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
use crate::mir::{
    MirBackendContract, MirEnumVariantLayout, MirNominalLayout, MirProgram, MirProjectionKey,
    MirVariantLayoutFields,
};
use crate::types::{AssociatedTypeKey, GenericParamId};
use inkwell::context::Context;

fn load_projection_contract(
    codegen: &mut CodeGen<'_>,
    mut type_context: crate::type_context::TypeContext,
    base: &Type,
    trait_id: DefId,
    assoc_type_id: AssocTypeId,
    trait_args: &[Type],
    output: &Type,
) {
    let base = type_context.intern_type(base);
    let trait_args = trait_args
        .iter()
        .map(|arg| type_context.intern_type(arg))
        .collect();
    let output = type_context.intern_type(output);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.projection_traits.insert(trait_id);
    backend_contract.projection_outputs.insert(
        MirProjectionKey {
            base,
            trait_id,
            assoc_type_id,
            trait_args,
        },
        output,
    );
    let mir = MirProgram {
        functions: BTreeMap::new(),
        type_context,
        backend_contract,
    };

    codegen.compile_program_from_mir(&mir).unwrap();
}

#[test]
fn codegen_lowers_projection_type_id_through_contract_resolution() {
    let context = inkwell::context::Context::create();
    let mut type_context = crate::type_context::TypeContext::new();
    let trait_id = DefId::new(CrateId(1), LocalDefId(1));
    let assoc_type_id = AssocTypeId(0);
    let base_ty = Type::Struct {
        id: DefId::new(CrateId(1), LocalDefId(2)),
        args: Vec::new(),
    };
    let trait_args = vec![Type::Bool];
    let projection = Type::Projection {
        ty: Box::new(base_ty.clone()),
        trait_id,
        assoc_type: AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id,
        },
        trait_args: trait_args.clone(),
    };
    let projection_id = type_context.intern_type(&projection);
    let mut codegen = CodeGen::new(&context, "projection_type_id_codegen_test");
    load_projection_contract(
        &mut codegen,
        type_context,
        &base_ty,
        trait_id,
        assoc_type_id,
        &trait_args,
        &Type::U8,
    );

    assert_eq!(
        codegen
            .llvm_type_id(projection_id)
            .unwrap()
            .into_int_type()
            .get_bit_width(),
        8
    );
}

#[test]
fn enum_payload_type_uses_codegen_type_id_layout() {
    let context = inkwell::context::Context::create();
    let mut type_context = crate::type_context::TypeContext::new();
    let owner = DefId::new(CrateId(1), LocalDefId(0));
    let enum_id = DefId::new(CrateId(1), LocalDefId(1));
    let generic_ty = Type::Generic(GenericParamId { owner, index: 0 });
    let generic_id = type_context.intern_type(&generic_ty);
    let mut codegen = CodeGen::new(&context, "enum_type_id_codegen_test");
    let mut backend_contract = MirBackendContract::default();
    backend_contract.nominal_layouts.insert(
        enum_id,
        MirNominalLayout::Enum {
            id: enum_id,
            variants: vec![MirEnumVariantLayout {
                name: "Some".to_string(),
                fields: MirVariantLayoutFields::Positional(vec![generic_id]),
            }],
            generic_params: vec![GenericParamId { owner, index: 0 }],
        },
    );
    let mir = MirProgram {
        functions: BTreeMap::new(),
        type_context,
        backend_contract,
    };
    codegen.compile_program_from_mir(&mir).unwrap();

    assert_eq!(
        codegen.enum_variant_payload_type_by_id(enum_id, &[Type::I64], 0),
        Some(Type::I64),
    );
}

#[test]
fn struct_substitution_by_id_uses_contract_generic_param_ids() {
    let context = inkwell::context::Context::create();
    let mut type_context = crate::type_context::TypeContext::new();
    let mut codegen = CodeGen::new(&context, "struct_generic_id_table_test");
    let owner = DefId::new(CrateId(1), LocalDefId(7));
    let first = GenericParamId { owner, index: 0 };
    let second = GenericParamId { owner, index: 1 };
    let i64_id = type_context.intern_type(&Type::I64);
    let bool_id = type_context.intern_type(&Type::Bool);
    let mut backend_contract = MirBackendContract::default();
    backend_contract.nominal_layouts.insert(
        owner,
        MirNominalLayout::Struct {
            id: owner,
            fields: vec![("left".to_string(), i64_id), ("right".to_string(), bool_id)],
            generic_params: vec![first, second],
        },
    );
    let mir = MirProgram {
        functions: BTreeMap::new(),
        type_context,
        backend_contract,
    };
    codegen.compile_program_from_mir(&mir).unwrap();

    let subst = codegen.struct_substitution_by_id(owner, &[Type::I64, Type::Bool]);

    assert_eq!(subst.get(&first), Some(&Type::I64));
    assert_eq!(subst.get(&second), Some(&Type::Bool));
}

#[test]
fn test_llvm_type_for_slice_reference_is_fat() {
    let context = Context::create();
    let codegen = CodeGen::new(&context, "test");

    let ty = codegen.llvm_type(&Type::Reference {
        mutable: false,
        inner: Box::new(Type::Slice(Box::new(Type::I64))),
    });

    assert!(matches!(ty.unwrap(), BasicTypeEnum::StructType(_)));
}

#[test]
fn test_llvm_type_for_str_reference_is_fat() {
    let context = inkwell::context::Context::create();
    let codegen = CodeGen::new(&context, "test");
    let ty = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Str),
    };

    assert!(codegen.llvm_type(&ty).unwrap().is_struct_type());
}

#[test]
fn test_llvm_type_for_sized_reference_stays_thin() {
    let context = Context::create();
    let codegen = CodeGen::new(&context, "test");

    let ty = codegen.llvm_type(&Type::Reference {
        mutable: false,
        inner: Box::new(Type::I64),
    });

    assert!(matches!(ty.unwrap(), BasicTypeEnum::PointerType(_)));
}

#[test]
fn test_unknown_nominal_struct_id_fails_explicitly() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    codegen.set_type_context(crate::type_context::TypeContext::new());

    let error = codegen.llvm_type(&Type::Struct {
        id: crate::ids::DefId::new(crate::ids::CrateId(99), crate::ids::LocalDefId(0)),
        args: Vec::new(),
    }).unwrap_err();
    assert!(error.message.contains("unknown nominal struct DefId"));
}

#[test]
fn resolve_projection_type_uses_contract_output_for_array_projection() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = crate::type_context::TypeContext::new();
    let trait_id = DefId::new(CrateId(1), LocalDefId(1));
    let assoc_type_id = AssocTypeId(0);
    let base_ty = Type::Array(Box::new(Type::I64), crate::types::ArrayLen::Value(3));
    let trait_args = vec![Type::I64];

    load_projection_contract(
        &mut codegen,
        type_context,
        &base_ty,
        trait_id,
        assoc_type_id,
        &trait_args,
        &Type::Bool,
    );

    let projection = Type::Projection {
        ty: Box::new(base_ty),
        trait_id,
        assoc_type: AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id,
        },
        trait_args,
    };

    assert_eq!(codegen.resolve_projection_type(&projection), Type::Bool);
}

#[test]
#[should_panic(expected = "missing MIR projection output")]
fn resolve_projection_type_requires_contract_output_for_array_projection() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    codegen.set_type_context(crate::type_context::TypeContext::new());
    let local_trait_id = DefId::new(CrateId(1), LocalDefId(10));
    let assoc_type_id = AssocTypeId(0);

    let projection = Type::Projection {
        ty: Box::new(Type::Array(Box::new(Type::I64), crate::types::ArrayLen::Value(3))),
        trait_id: local_trait_id,
        assoc_type: AssociatedTypeKey {
            owner: local_trait_id,
            assoc_type_id,
        },
        trait_args: vec![Type::I64],
    };

    let _ = codegen.resolve_projection_type(&projection);
}

#[test]
fn resolve_projection_type_uses_contract_output_without_projection_impls() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    let type_context = crate::type_context::TypeContext::new();
    let trait_id = DefId::new(CrateId(1), LocalDefId(10));
    let assoc_type_id = AssocTypeId(0);
    let base_ty = Type::Struct {
        id: DefId::new(CrateId(1), LocalDefId(11)),
        args: vec![Type::I64],
    };
    let trait_args = vec![Type::Bool];

    load_projection_contract(
        &mut codegen,
        type_context,
        &base_ty,
        trait_id,
        assoc_type_id,
        &trait_args,
        &Type::U8,
    );

    let projection = Type::Projection {
        ty: Box::new(base_ty),
        trait_id,
        assoc_type: AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id,
        },
        trait_args,
    };

    assert_eq!(codegen.resolve_projection_type(&projection), Type::U8);
}

#[test]
#[should_panic(expected = "missing MIR projection output")]
fn resolve_projection_type_requires_contract_output_for_named_projection() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    codegen.set_type_context(crate::type_context::TypeContext::new());
    let trait_id = DefId::new(CrateId(1), LocalDefId(20));
    let assoc_type_id = AssocTypeId(0);
    let base_ty = Type::Struct {
        id: DefId::new(CrateId(1), LocalDefId(22)),
        args: vec![Type::I64],
    };

    let projection = Type::Projection {
        ty: Box::new(base_ty),
        trait_id,
        assoc_type: AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id,
        },
        trait_args: Vec::new(),
    };

    let _ = codegen.resolve_projection_type(&projection);
}

#[test]
fn resolve_projection_type_rejects_assoc_type_owned_by_different_trait() {
    let context = Context::create();
    let mut codegen = CodeGen::new(&context, "test");
    codegen.set_type_context(crate::type_context::TypeContext::new());
    let trait_id = DefId::new(CrateId(1), LocalDefId(10));
    let other_trait_id = DefId::new(CrateId(1), LocalDefId(11));
    let assoc_type_id = AssocTypeId(0);
    let base_ty = Type::Struct {
        id: DefId::new(CrateId(1), LocalDefId(13)),
        args: Vec::new(),
    };

    let projection = Type::Projection {
        ty: Box::new(base_ty),
        trait_id,
        assoc_type: AssociatedTypeKey {
            owner: other_trait_id,
            assoc_type_id,
        },
        trait_args: vec![],
    };

    assert!(matches!(
        codegen.resolve_projection_type(&projection),
        Type::Projection { .. }
    ));
}

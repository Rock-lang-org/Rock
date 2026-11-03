use super::*;
use crate::collect::resolver::ResolverTables;
use crate::hir::{HirBlock, HirExpr, HirExprKind, HirFunction, HirImpl, HirImplOwner, HirParam};
use crate::ids::{CrateId, DefId, LocalDefId, TypeVarId};
use crate::lexer::Span;
use crate::types::ReceiverMode;
use crate::types::{GenericParamDecl, GenericParamId, Type};

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

fn receiver_candidate(expr: HirExpr) -> ReceiverCandidate {
    ReceiverCandidate {
        expr,
        adjustment: ReceiverAdjustment::None,
        can_autoref_mut: false,
    }
}

fn method(id: DefId, owner: DefId) -> HirFunction {
    HirFunction {
        id,
        name: "value".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![HirParam {
            name: "self".to_string(),
            local_id: crate::ids::HirLocalId(0),
            ty: Type::Struct {
                id: owner,
                args: Vec::new(),
            },
            mutable: false,
            is_ref: false,
        }],
        ret_type: Type::I64,
        body: HirBlock {
            stmts: Vec::new(),
            ty: Type::I64,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Shared),
        is_unsafe: false,
    }
}

fn operator_method(id: DefId, owner: DefId, ret_type: Type) -> HirFunction {
    HirFunction {
        id,
        name: "+".to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: vec![
            HirParam {
                name: "self".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Struct {
                    id: owner,
                    args: Vec::new(),
                },
                mutable: false,
                is_ref: false,
            },
            HirParam {
                name: "rhs".to_string(),
                local_id: crate::ids::HirLocalId(0),
                ty: Type::Struct {
                    id: owner,
                    args: Vec::new(),
                },
                mutable: false,
                is_ref: false,
            },
        ],
        ret_type: ret_type.clone(),
        body: HirBlock {
            stmts: Vec::new(),
            ty: ret_type,
        },
        is_curried: false,
        is_method: true,
        self_receiver: Some(ReceiverMode::Shared),
        is_unsafe: false,
    }
}

fn register_trait_member(
    traits: &mut HashMap<DefId, crate::hir::HirTrait>,
    imp: &HirImpl,
    method_name: &str,
) -> DefId {
    let trait_id = imp.trait_id.expect("test trait impl");
    let trait_name = imp.trait_name.clone().expect("test trait name");
    let method = imp.methods.get(method_name).expect("test impl method");
    let member_id = DefId::new(
        method.id.crate_id,
        LocalDefId(
            method
                .id
                .local
                .0
                .checked_add(10_000)
                .expect("test member ID overflow"),
        ),
    );
    let trait_def = traits
        .entry(trait_id)
        .or_insert_with(|| crate::hir::HirTrait {
            target: None,
            predicates: Vec::new(),
            id: trait_id,
            name: trait_name,
            generic_params: imp.trait_generics.clone(),
            associated_types: imp
                .associated_types
                .iter()
                .map(|assoc| crate::hir::HirAssociatedTypeDecl {
                    id: assoc.id,
                    name: assoc.name.clone(),
                    kind: assoc.kind.clone(),
                })
                .collect(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        });
    assert_eq!(trait_def.id, trait_id);
    trait_def.signatures.insert(
        method_name.to_string(),
        crate::hir::HirFunctionSig {
            id: member_id,
            name: method_name.to_string(),
            generic_params: method.generic_params.clone(),
            params: method.params.iter().map(|param| param.ty.clone()).collect(),
            ret: method.ret_type.clone(),
            generic_bounds: method.generic_bounds.clone(),
            self_receiver: method.self_receiver,
            is_unsafe: method.is_unsafe,
        },
    );
    member_id
}

fn renamed_trait_member_fixture(
    trait_id: DefId,
    member_id: DefId,
    source_name: &str,
    impl_id: DefId,
    impl_method_id: DefId,
) -> (HashMap<DefId, HirTrait>, HashMap<DefId, HirImpl>, HirExpr) {
    let owner_id = def_id(9);
    let receiver_ty = Type::Struct {
        id: owner_id,
        args: Vec::new(),
    };
    let mut trait_method = method(member_id, owner_id);
    trait_method.name = source_name.to_string();
    let trait_def = HirTrait {
        target: None,
        predicates: Vec::new(),
        id: trait_id,
        name: "RenamedProtocol".to_string(),
        generic_params: Vec::new(),
        associated_types: Vec::new(),
        methods: HashMap::from([(source_name.to_string(), trait_method)]),
        signatures: HashMap::new(),
    };

    let mut impl_method = method(impl_method_id, owner_id);
    impl_method.name = source_name.to_string();
    let imp = HirImpl {
        id: impl_id,
        owner: HirImplOwner::Named("Owner".to_string()),
        type_name: "Owner".to_string(),
        type_generics: Vec::new(),
        receiver_pattern: HirImplReceiverPattern::Exact(receiver_ty.clone()),
        trait_name: Some("RenamedProtocol".to_string()),
        trait_id: Some(trait_id),
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods: HashMap::from([(source_name.to_string(), impl_method)]),
    };
    let receiver = HirExpr {
        kind: HirExprKind::Var("owner".to_string()),
        ty: receiver_ty,
        span: Span::test(),
    };

    (
        HashMap::from([(trait_id, trait_def)]),
        HashMap::from([(impl_id, imp)]),
        receiver,
    )
}

fn effective_trait_methods_for(
    traits: &HashMap<DefId, HirTrait>,
    impls: &HashMap<DefId, HirImpl>,
) -> HashMap<(DefId, DefId), DefId> {
    impls
        .values()
        .filter_map(|imp| {
            let trait_id = imp.trait_id?;
            let trait_def = traits.get(&trait_id)?;
            Some(
                imp.methods
                    .iter()
                    .filter_map(|(name, method)| {
                        let member_id = trait_def
                            .methods
                            .get(name)
                            .map(|member| member.id)
                            .or_else(|| {
                                trait_def.signatures.get(name).map(|signature| signature.id)
                            })?;
                        Some(((imp.id, member_id), method.id))
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .flatten()
        .collect()
}

mod bounds;
mod constructor_traits;
mod impls;
mod operators;
mod receivers;
mod trait_members;

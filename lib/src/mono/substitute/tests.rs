use std::collections::HashMap;

use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId};
use crate::type_context::{Ty, TypeContext};
use crate::types::{AssociatedTypeKey, GenericParamId, Type};

fn def_id(local: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(local))
}

#[test]
fn type_id_substitution_rewrites_generic_projection_args() {
    let owner = def_id(90);
    let trait_id = def_id(91);
    let generic = GenericParamId { owner, index: 0 };
    let assoc = AssociatedTypeKey {
        owner: trait_id,
        assoc_type_id: AssocTypeId(0),
    };
    let mut context = TypeContext::new();
    let base = context.intern_ty(Ty::Generic(generic));
    let projection = context.intern_ty(Ty::Projection {
        ty: base,
        trait_id,
        assoc_type: assoc,
        trait_args: vec![base],
    });
    let replacement = context.intern_type(&Type::Struct {
        id: def_id(92),
        args: Vec::new(),
    });
    let mut subst = HashMap::new();
    subst.insert(generic, replacement);

    let rewritten = context.substitute_generics(projection, &subst);

    assert_eq!(
        context.type_for(rewritten),
        Type::Projection {
            ty: Box::new(Type::Struct {
                id: def_id(92),
                args: Vec::new(),
            }),
            trait_id,
            assoc_type: assoc,
            trait_args: vec![Type::Struct {
                id: def_id(92),
                args: Vec::new(),
            }],
        }
    );
}

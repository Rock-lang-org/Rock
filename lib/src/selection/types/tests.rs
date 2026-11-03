use super::*;
use crate::hir::HirMethodCallTarget;
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::types::Type;

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

#[test]
fn selection_diagnostic_preserves_selected_target_identity() {
    let target = HirMethodCallTarget::impl_method(
        def_id(20),
        def_id(30),
        Some(crate::hir::HirSelectedTraitMember {
            trait_id: def_id(10),
            member_id: def_id(30),
            trait_args: vec![Type::I64],
        }),
    );
    let diagnostic = SelectionDiagnostic::SelectedTargetMissing {
        method_name: "show".to_string(),
        target: target.clone(),
    };

    assert_eq!(
        diagnostic.message(),
        "Selected method target for 'show' could not be resolved by identity"
    );
    assert_eq!(diagnostic.target(), Some(&target));
}

#[test]
fn default_selection_message_hides_internal_type_ids() {
    let diagnostic = SelectionDiagnostic::NoImplementation {
        operation: "+".to_string(),
        receiver: Type::Struct {
            id: def_id(42),
            args: vec![Type::I64],
        },
    };

    let message = diagnostic.message();

    assert_eq!(
        message,
        "No implementation found for operator '+' on type <unknown type> I64"
    );
    assert!(!message.contains("struct#"));
    assert!(!message.contains("DefId"));
}

#[test]
fn selected_method_authority_records_impl_trait_method_and_args() {
    let impl_id = def_id(20);
    let trait_id = def_id(10);
    let method_id = def_id(30);
    let target = HirMethodCallTarget::impl_method(
        impl_id,
        method_id,
        Some(crate::hir::HirSelectedTraitMember {
            trait_id,
            member_id: method_id,
            trait_args: vec![Type::I64],
        }),
    );
    let selected = SelectedMethod {
        receiver: HirExpr {
            kind: crate::hir::HirExprKind::Var("value".to_string()),
            ty: Type::I64,
            span: crate::Span::test(),
        },
        function: None,
        impl_def: None,
        target,
        origin: SelectedOrigin::TraitImpl { impl_id, trait_id },
        receiver_adjustment: ReceiverAdjustment::BuiltinDeref,
        substituted_params: Vec::new(),
        return_type: Type::Bool,
        pending_impl_bounds: Vec::new(),
        associated_types: Vec::new(),
        owner_substitution: HashMap::new(),
        owner_generic_params: Vec::new(),
    };

    let authority = selected.authority();

    assert_eq!(authority.target, selected.target.target);
    assert_eq!(
        authority.receiver_adjustment,
        ReceiverAdjustment::BuiltinDeref
    );
    assert_eq!(authority.return_type, Type::Bool);
}

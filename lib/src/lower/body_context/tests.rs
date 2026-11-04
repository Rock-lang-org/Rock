use super::*;

use crate::ids::{CrateId, LocalDefId};

#[test]
fn body_context_can_continue_after_existing_parameter_ids() {
    let mut context = BodyLoweringContext::new(
        BodyOwner::Function(DefId::new(CrateId(0), LocalDefId(1))),
        None,
        Vec::new(),
        HirGenericBounds::new(),
        false,
    );

    context.seed_after_existing_locals([HirLocalId(0), HirLocalId(2)]);

    assert_eq!(context.fresh_local_id(), HirLocalId(3));
    assert_eq!(context.fresh_local_id(), HirLocalId(4));
}

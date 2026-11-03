use super::*;

#[test]
fn id_gen_allocates_typed_sequential_ids() {
    let mut gen = IdGen::<ModuleId>::new();

    let first = gen.fresh();
    let second = gen.fresh();

    assert_eq!(first.raw(), 0);
    assert_eq!(second.raw(), 1);
    assert_eq!(second.index(), 1);
    assert_eq!(gen.next_raw(), 2);
}

#[test]
#[should_panic(expected = "compiler ID generator exhausted u32 ID space")]
fn id_gen_panics_when_id_space_is_exhausted() {
    let mut gen = IdGen::<ModuleId> {
        next: u32::MAX,
        _marker: PhantomData,
    };

    let _ = gen.fresh();
}

#[test]
fn def_id_carries_crate_and_local_identity() {
    let def = DefId::new(CrateId(3), LocalDefId(42));

    assert_eq!(def.crate_id, CrateId(3));
    assert_eq!(def.local, LocalDefId(42));
}

#[test]
fn type_var_id_preserves_existing_raw_type_var_numbers() {
    let id = TypeVarId::from_raw(17);

    assert_eq!(id.raw(), 17);
    assert_eq!(id.index(), 17);
}

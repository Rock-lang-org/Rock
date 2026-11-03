use super::*;
use crate::ids::Idx;
use crate::mir::{Local, Place, Projection};

fn local(local: usize) -> Place {
    Place {
        local: Local(local),
        projection: Vec::new(),
    }
}

fn field(local: usize, index: usize) -> Place {
    Place {
        local: Local(local),
        projection: vec![Projection::Field {
            index,
            identity: None,
        }],
    }
}

#[test]
fn place_path_table_interns_places_once_and_records_parent() {
    let mut table = PlacePathTable::new();

    let root = table.intern(local(1));
    let field0 = table.intern(field(1, 0));
    let field0_again = table.intern(field(1, 0));

    assert_eq!(root.raw(), 0);
    assert_eq!(field0, field0_again);
    assert_eq!(table.get(field0).unwrap().parent, Some(root));
    assert_eq!(table.place(field0), Some(&field(1, 0)));
}

#[test]
fn place_path_table_distinguishes_disjoint_fields() {
    let mut table = PlacePathTable::new();
    let field0 = table.intern(field(1, 0));
    let field1 = table.intern(field(1, 1));

    assert!(!table.paths_conflict(field0, field1));
    let root = table.intern(local(1));
    assert!(table.paths_conflict(root, field0));
}

#[test]
fn place_path_table_collects_function_places() {
    use crate::mir::{
        BasicBlock, LocalDecl, MirFunction, MirFunctionId, Mutability, Operand, Rvalue,
        StatementData,
    };

    let mut type_context = crate::type_context::TypeContext::new();
    let i64_id = type_context.intern_type(&crate::types::Type::I64);
    let function = MirFunction {
        id: MirFunctionId::Function(crate::ids::DefId::new(
            crate::ids::CrateId(0),
            crate::ids::LocalDefId(0),
        )),
        name: "scan".to_string(),
        basic_blocks: vec![BasicBlock {
            statements: vec![StatementData::assign(
                field(0, 1),
                Rvalue::Use(Operand::Copy(field(1, 0))),
                None,
            )],
            terminator: None,
        }],
        local_decls: vec![
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("a".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
            LocalDecl {
                ty: i64_id,
                mutability: Mutability::Mut,
                name: Some("b".to_string()),
                span: None,
                source: crate::mir::LocalSource::UserBinding,
            },
        ],
        closure_captures: Vec::new(),
        arg_count: 0,
        ret_type: i64_id,
        ownership: Default::default(),
    };

    let table = PlacePathTable::for_function(&function);

    assert!(table.path_id(&local(0)).is_some());
    assert!(table.path_id(&field(0, 1)).is_some());
    assert!(table.path_id(&field(1, 0)).is_some());
}

#[test]
fn move_path_table_maps_to_place_paths_and_tracks_children() {
    let mut places = PlacePathTable::new();
    let root_place = places.intern(local(2));
    let child_place = places.intern(field(2, 0));
    let moves = MovePathTable::from_place_paths(&places);

    let root_move = moves.move_path_for_place(root_place).unwrap();
    let child_move = moves.move_path_for_place(child_place).unwrap();

    assert_eq!(moves.place_path(root_move), Some(root_place));
    assert!(moves.is_ancestor(root_move, child_move));
    assert!(!moves.is_ancestor(child_move, root_move));
}

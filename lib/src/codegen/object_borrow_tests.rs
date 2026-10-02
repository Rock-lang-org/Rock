use crate::ids::InstanceId;
use crate::mir::*;
use crate::types::Type;

use super::{local, object_program, place, pointee};

#[test]
fn object_loans_survive_copy_move_conversions_and_block_boundaries() {
    for (mutable, copy) in [(false, false), (false, true), (true, false)] {
        for split in [None, Some(2), Some(3), Some(4)] {
            let mut program = object_program(mutable);
            let caller = program
                .functions
                .get_mut(&MirFunctionId::Instance(InstanceId(0)))
                .unwrap();
            if copy {
                for statement in &mut caller.basic_blocks[0].statements {
                    if let StatementKind::Assign(_, Rvalue::Object(operand, _)) =
                        &mut statement.kind
                    {
                        let Operand::Move(source) = operand else {
                            unreachable!()
                        };
                        *operand = Operand::Copy(source.clone());
                    }
                }
            }
            let call_block = if let Some(split) = split {
                let statements = caller.basic_blocks[0].statements.split_off(split);
                let terminator = caller.basic_blocks[0].terminator.take();
                caller.basic_blocks[0].terminator = Some(Terminator::Goto(BasicBlockId(2)));
                caller.basic_blocks.push(BasicBlock {
                    statements,
                    terminator,
                });
                2
            } else {
                0
            };
            crate::mir::borrowck::BorrowChecker::run(&program).unwrap();
            let caller = program
                .functions
                .get_mut(&MirFunctionId::Instance(InstanceId(0)))
                .unwrap();
            caller.basic_blocks[call_block]
                .statements
                .push(StatementData::assign(
                    place(1),
                    Rvalue::Use(Operand::Constant(Constant::Int(100))),
                    None,
                ));
            let error = crate::mir::borrowck::BorrowChecker::run(&program)
                .expect_err("live object must retain its payload loan");
            assert!(
                error
                    .0
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains("borrow")),
                "mutable={mutable}, copy={copy}, split={split:?}: {error:?}"
            );
        }
    }
}

#[test]
fn concrete_and_upcast_operands_cannot_extract_unique_references_through_shared_access() {
    for conversion_index in [2, 3] {
        for copy in [false, true] {
            let mut program = object_program(true);
            let caller_id = MirFunctionId::Instance(InstanceId(0));
            let source_local = if conversion_index == 2 { 2 } else { 3 };
            let source_ty = program
                .type_context
                .type_for(program.functions[&caller_id].local_decls[source_local].ty);
            let shared = program.type_context.intern_type(&Type::Reference {
                inner: Box::new(source_ty),
                mutable: false,
            });
            let caller = program.functions.get_mut(&caller_id).unwrap();
            caller
                .local_decls
                .push(local(shared, LocalSource::Temporary));
            let StatementKind::Assign(_, Rvalue::Object(operand, _)) =
                &mut caller.basic_blocks[0].statements[conversion_index].kind
            else {
                unreachable!()
            };
            *operand = if copy {
                Operand::Copy(pointee(5))
            } else {
                Operand::Move(pointee(5))
            };
            caller.basic_blocks[0].statements.insert(
                conversion_index,
                StatementData::assign(
                    place(5),
                    Rvalue::Ref(Mutability::Not, place(source_local)),
                    None,
                ),
            );
            let error = crate::mir::borrowck::BorrowChecker::run(&program)
                .expect_err("object conversion must validate projected copy/move access");
            assert!(
                error
                    .0
                    .iter()
                    .any(|diagnostic| diagnostic.message.contains("non-copy")
                        && diagnostic.message.contains("reference")),
                "copy={copy}, conversion={conversion_index}: {error:?}"
            );
        }
    }
}

#[test]
fn reborrow_into_object_keeps_the_original_loan_until_the_virtual_use() {
    let mut program = object_program(false);
    let id = MirFunctionId::Instance(InstanceId(0));
    let caller = program.functions.get_mut(&id).unwrap();
    caller
        .local_decls
        .push(local(caller.local_decls[2].ty, LocalSource::Temporary));
    let StatementKind::Assign(_, Rvalue::Object(operand, _)) =
        &mut caller.basic_blocks[0].statements[2].kind
    else {
        unreachable!()
    };
    *operand = Operand::Move(place(5));
    caller.basic_blocks[0].statements.insert(
        2,
        StatementData::assign(place(5), Rvalue::Ref(Mutability::Not, pointee(2)), None),
    );
    // The same write is legal once the scalar-returning virtual call is over.
    caller.basic_blocks[1]
        .statements
        .push(StatementData::assign(
            place(1),
            Rvalue::Use(Operand::Constant(Constant::Int(100))),
            None,
        ));
    crate::mir::borrowck::BorrowChecker::run(&program).unwrap();
    let caller = program.functions.get_mut(&id).unwrap();
    caller.basic_blocks[0]
        .statements
        .push(StatementData::assign(
            place(1),
            Rvalue::Use(Operand::Constant(Constant::Int(100))),
            None,
        ));
    let error = crate::mir::borrowck::BorrowChecker::run(&program)
        .expect_err("reborrow must preserve the original payload loan");
    assert!(
        error
            .0
            .iter()
            .any(|diagnostic| diagnostic.message.contains("borrow")),
        "{error:?}"
    );
}

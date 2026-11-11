use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashSet};

use crate::ids::TypeId;
use crate::mir::{
    Constant, Local, LocalSource, MirBinOp, MirCallable, MirCallableKey, MirFunction,
    MirFunctionId, MirOrigin, MirRuntimeHelper, MirSyntheticOrigin, Mutability, Operand, Place,
    Projection, Rvalue, StatementData, Terminator,
};
use crate::type_context::TypeContext;
use crate::types::{ArrayLen, Type};

use super::drop_state::LoopOwnedArray;
use super::{HirProgram, MirBuilder};

fn place(local: Local) -> Place {
    Place { local, projection: Vec::new() }
}

// Everything built here is drop glue (an early-exit scope cleanup or the tail
// helper), so even the index increment and return place write use
// `cleanup_assign`. The flag only tags the origin as synthetic cleanup and
// exempts discriminant reads from init checks; none of these statements read
// one, so using it for non-cleanup-state writes is a labelling choice, not a
// semantic requirement.
impl<'a> MirBuilder<'a> {
    /// The binding already owns `counter`; transfer the remaining elements to
    /// one type-keyed helper without merging the caller's cleanup paths.
    pub(super) fn emit_loop_owned_array_early_exit(
        &mut self,
        temp: Local,
        owned: LoopOwnedArray,
        end_storage: bool,
    ) {
        if self.current_block.is_none() {
            return;
        }
        let elem_id = self.type_id_for(&owned.elem_ty);
        let array_ty = Type::Array(
            Box::new(owned.elem_ty.clone()),
            ArrayLen::Value(owned.len as u64),
        );
        let ref_id = self.type_id_for(&Type::Reference {
            mutable: false,
            inner: Box::new(array_ty),
        });
        let ptr_id = self.type_id_for(&Type::Pointer(Box::new(owned.elem_ty)));
        let reference = self.new_local(ref_id, Mutability::Not, None);
        let pointer = self.new_local(ptr_id, Mutability::Not, None);
        let start = self.new_local(self.type_id_for(&Type::I64), Mutability::Not, None);
        let result = self.new_local(self.type_id_for(&Type::Unit), Mutability::Not, None);
        let next = self.new_block();
        if let Some(current) = self.current_block {
            let statements = &mut self.blocks[current.0].statements;
            for local in [reference, pointer, start, result] {
                statements.push(StatementData::storage_live(local, None));
            }
            statements.push(StatementData::cleanup_assign(
                place(reference), Rvalue::Ref(Mutability::Not, place(temp)), None,
            ));
            // An array and its first element have the same address, including
            // empty arrays. No element is accessed until start < len.
            statements.push(StatementData::cleanup_assign(
                place(pointer), Rvalue::Cast(Operand::Copy(place(reference)), ptr_id), None,
            ));
            statements.push(StatementData::cleanup_assign(
                place(start),
                Rvalue::BinaryOp(
                    MirBinOp::Add,
                    Operand::Copy(place(owned.counter)),
                    Operand::Constant(Constant::Int(1)),
                ),
                None,
            ));
            statements.push(StatementData::storage_dead(reference, None));
            self.blocks[current.0].terminator = Some(Terminator::Call {
                func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                    MirCallableKey::RuntimeHelper(MirRuntimeHelper::ArrayTailDrop(elem_id)),
                ))),
                args: vec![
                    Operand::Copy(place(pointer)),
                    Operand::Copy(place(start)),
                    Operand::Constant(Constant::Int(owned.len as i64)),
                ],
                destination: place(result),
                target: next,
                span: None,
            });
        }
        self.current_block = Some(next);
        for local in [result, start, pointer] {
            self.blocks[next.0].statements.push(StatementData::storage_dead(local, None));
        }
        if end_storage {
            self.end_loop_owned_array_storage(temp);
        }
    }

    /// Materialize helpers after instance DCE: their drop callees are already
    /// runtime roots, and only helpers actually referenced by retained bodies
    /// need emitting. TypeId identity shares a helper across lengths/functions.
    pub(super) fn populate_array_tail_helpers(
        program: &HirProgram,
        types: &RefCell<TypeContext>,
        direct_drop_types: &HashSet<Type>,
        functions: &mut BTreeMap<MirFunctionId, MirFunction>,
    ) -> Result<(), crate::diagnostic::Diagnostics> {
        let elements = functions.values().flat_map(|function| {
            function.basic_blocks.iter().filter_map(|block| match &block.terminator {
                Some(Terminator::Call {
                    func: Operand::Constant(Constant::Callable(MirCallable::Resolved(
                        MirCallableKey::RuntimeHelper(MirRuntimeHelper::ArrayTailDrop(ty)),
                    ))),
                    ..
                }) => Some(*ty),
                _ => None,
            })
        }).collect::<BTreeSet<_>>();
        for elem in elements {
            let mut builder = MirBuilder::with_instance_metadata(
                program, types, Default::default(), direct_drop_types.clone(),
            );
            let helper = builder.build_array_tail_helper(elem);
            builder.diagnostics.return_if_err()?;
            functions.insert(helper.id.clone(), helper);
        }
        Ok(())
    }

    pub(super) fn build_array_tail_helper(&mut self, elem: TypeId) -> MirFunction {
        let id = MirFunctionId::ArrayTailDrop(elem);
        self.current_function_id = Some(id.clone());
        let elem_ty = self.type_context.borrow().type_for(elem);
        let unit = self.type_id_for(&Type::Unit);
        let int = self.type_id_for(&Type::I64);
        let pointer = self.type_id_for(&Type::Pointer(Box::new(elem_ty.clone())));
        self.new_local_with_source(unit, Mutability::Mut, None, LocalSource::ReturnPlace, None);
        let data = self.new_local_with_source(pointer, Mutability::Not, None, LocalSource::Argument, None);
        let index = self.new_local_with_source(int, Mutability::Mut, None, LocalSource::Argument, None);
        let len = self.new_local_with_source(int, Mutability::Not, None, LocalSource::Argument, None);
        let condition = self.new_local(self.type_id_for(&Type::Bool), Mutability::Not, None);
        let entry = self.new_block();
        let head = self.new_block();
        let body = self.new_block();
        let done = self.new_block();
        self.current_block = Some(entry);
        let origin = MirOrigin::synthetic(MirSyntheticOrigin::Cleanup, None);
        self.blocks[entry.0].statements.push(StatementData::storage_live(condition, None));
        self.blocks[entry.0].terminator = Some(Terminator::goto(head, origin.clone()));
        self.blocks[head.0].statements.push(StatementData::cleanup_assign(
            place(condition),
            Rvalue::BinaryOp(MirBinOp::Lt, Operand::Copy(place(index)), Operand::Copy(place(len))),
            None,
        ));
        self.blocks[head.0].terminator = Some(Terminator::switch_int(
            Operand::Copy(place(condition)), vec![(1, body)], done, origin.clone(),
        ));
        self.current_block = Some(body);
        let previous_cleanup = self.drops.enter_cleanup();
        // Reuse the complete drop elaborator: direct Drop first, then fields,
        // tuple/array elements, enum payloads and closure cleanup as usual.
        self.emit_drop_for_place(&elem_ty, Place {
            local: data,
            projection: vec![Projection::Index(index)],
        }, None);
        self.drops.leave_cleanup(previous_cleanup);
        if let Some(current) = self.current_block {
            self.blocks[current.0].statements.push(StatementData::cleanup_assign(
                place(index),
                Rvalue::BinaryOp(MirBinOp::Add, Operand::Copy(place(index)), Operand::Constant(Constant::Int(1))),
                None,
            ));
            self.blocks[current.0].terminator = Some(Terminator::goto(head, origin));
        }
        self.blocks[done.0].statements.push(StatementData::storage_dead(condition, None));
        self.blocks[done.0].statements.push(StatementData::cleanup_assign(
            place(Local(0)), Rvalue::Use(Operand::Constant(Constant::Unit)), None,
        ));
        self.blocks[done.0].terminator = Some(Terminator::Return);
        MirFunction {
            id,
            name: format!("__rock_drop_array_tail_{}", elem.0),
            basic_blocks: std::mem::take(&mut self.blocks),
            local_decls: std::mem::take(&mut self.locals),
            closure_captures: Vec::new(),
            arg_count: 3,
            ret_type: unit,
            ownership: std::mem::take(&mut self.ownership),
        }
    }
}

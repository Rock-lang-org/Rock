//! Exact structural partitions for erased partial moves and conditional cleanup.
use crate::type_context::TypeContext;

use super::{erased_place_type, nominal_requirement, MirBackendContract, MirErasedError,
    MirErasedFunction, MirErasedLocal, MirErasedOperand, MirErasedPlace, MirErasedProjection,
    MirErasedRvalue, MirErasedTerminator, MirErasedType, MirNominalDropAuthority};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirErasedMovePaths {
    pub places: Vec<MirErasedPlace>,
    pub children: Vec<Vec<usize>>,
    pub parents: Vec<Option<usize>>,
}

impl MirErasedMovePaths {
    pub fn roots(function: &MirErasedFunction) -> Self {
        let count = function.body.as_ref().map_or(0, |body| body.locals.len());
        Self {
            places: (0..count).map(|index| MirErasedPlace::local(MirErasedLocal(index as u32))).collect(),
            children: vec![vec![]; count],
            parents: vec![None; count],
        }
    }

    pub fn exact(&self, place: &MirErasedPlace) -> Option<usize> {
        self.places.iter().position(|candidate| candidate == place)
    }

    pub fn containing(&self, place: &MirErasedPlace) -> usize {
        self.places.iter().enumerate().filter(|(_, candidate)| {
            candidate.local == place.local && place.projection.starts_with(&candidate.projection)
        }).max_by_key(|(_, candidate)| candidate.projection.len())
            .map_or(place.local.0 as usize, |(index, _)| index)
    }

    pub fn descendants(&self, index: usize) -> Vec<usize> {
        let mut result = vec![index];
        let mut next = 0;
        while next < result.len() {
            result.extend(&self.children[result[next]]);
            next += 1;
        }
        result
    }

    pub fn affected(&self, index: usize, initializing: bool) -> Vec<usize> {
        let mut result = self.descendants(index);
        if !initializing {
            let mut parent = self.parents[index];
            while let Some(index) = parent {
                result.push(index);
                parent = self.parents[index];
            }
        }
        result
    }
}

pub(crate) fn operands(value: &MirErasedRvalue) -> Vec<&MirErasedOperand> {
    match value {
        MirErasedRvalue::Forget(value) | MirErasedRvalue::Use(value)
        | MirErasedRvalue::DropInPlace { pointer: value, .. }
        | MirErasedRvalue::PackageOpen { source: value, .. }
        | MirErasedRvalue::Open { source: value, .. }
        | MirErasedRvalue::OwnerOpen { source: value, .. }
        | MirErasedRvalue::Reclose { value, .. }
        | MirErasedRvalue::PackagePack { payload: value, .. } => vec![value],
        MirErasedRvalue::NominalAggregate { fields, .. } => fields.iter().map(|(_, operand)| operand).collect(),
        MirErasedRvalue::Aggregate(args) | MirErasedRvalue::NativeAggregate { fields: args, .. }
        | MirErasedRvalue::Call { args, .. } | MirErasedRvalue::NativeCall { args, .. }
        | MirErasedRvalue::NativeApply { args, .. } | MirErasedRvalue::OpenedCall { args, .. }
        | MirErasedRvalue::OpenedErasedCall { args, .. }
        | MirErasedRvalue::DictionaryCall { args, .. } => args.iter().collect(),
        MirErasedRvalue::OwnerCall { owner, args, .. } => std::iter::once(owner).chain(args).collect(),
        MirErasedRvalue::Borrow { .. } | MirErasedRvalue::Literal(_) => vec![],
    }
}

pub fn collect_erased_move_paths(
    function: &MirErasedFunction,
    contract: &MirBackendContract,
    tc: &TypeContext,
) -> Result<MirErasedMovePaths, MirErasedError> {
    let mut paths = MirErasedMovePaths::roots(function);
    let Some(body) = &function.body else { return Ok(paths); };
    let moves = body.blocks.iter().flat_map(|block| {
        block.statements.iter().flat_map(|statement| operands(&statement.value))
            .chain(match &block.terminator {
                MirErasedTerminator::Return(value) | MirErasedTerminator::Branch { condition: value, .. } => Some(value),
                _ => None,
            })
    }).filter_map(|operand| match operand {
        MirErasedOperand::Move(place) if !place.projection.is_empty() => Some(place),
        _ => None,
    });
    for place in moves {
        if place.projection.len() > 128 || place.projection.contains(&MirErasedProjection::Deref)
            || (place.local.0 == 0 && contract.erased.nominal_drop_entries.values().any(|entry| entry.entry == function.key))
        {
            return Err(MirErasedError::ProjectedMoveRequiresDropProof);
        }
        let mut prefix = MirErasedPlace::local(place.local);
        let mut node = paths.exact(&prefix).ok_or(MirErasedError::InvalidLocal)?;
        for projection in &place.projection {
            if paths.children[node].is_empty() {
                let ty = erased_place_type(function, &prefix, contract, tc)?;
                let fields = match ty {
                    MirErasedType::Tuple(fields) => (0..fields.len()).map(|index| MirErasedProjection::Field(index as u32)).collect::<Vec<_>>(),
                    MirErasedType::Nominal { id, .. }
                        if contract.nominal_drop_authority.get(&id) == Some(&MirNominalDropAuthority::FieldsOnly) => {
                        nominal_requirement(contract, tc, &ty)?.fields.into_iter().map(|(field, _)| {
                            MirErasedProjection::NominalField { owner: id, field }
                        }).collect()
                    }
                    _ => return Err(MirErasedError::ProjectedMoveRequiresDropProof),
                };
                for field in fields {
                    if paths.places.len() >= 65536 { return Err(MirErasedError::UnsupportedOperation); }
                    let mut child = prefix.clone();
                    child.projection.push(field);
                    let index = paths.places.len();
                    paths.places.push(child);
                    paths.children.push(vec![]);
                    paths.parents.push(Some(node));
                    paths.children[node].push(index);
                }
            }
            prefix.projection.push(projection.clone());
            node = paths.exact(&prefix).ok_or(MirErasedError::ProjectedMoveRequiresDropProof)?;
        }
    }
    Ok(paths)
}

pub fn validated_erased_move_paths(
    function: &MirErasedFunction,
    contract: &MirBackendContract,
    tc: &TypeContext,
) -> Result<MirErasedMovePaths, MirErasedError> {
    let roots = MirErasedMovePaths::roots(function);
    let Some(supplied) = contract.erased.move_paths.get(&function.key) else { return Ok(roots); };
    if *supplied != collect_erased_move_paths(function, contract, tc)? || *supplied == roots {
        return Err(MirErasedError::ProjectedMoveRequiresDropProof);
    }
    Ok(supplied.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::FieldId;
    use crate::mir::{validate_erased_function, MirErasedAssignment, MirErasedLiteral, MirErasedKey, MirProgram};

    fn fixture() -> (MirProgram, MirErasedKey, MirErasedPlace) {
        let (mut program, key, owner) = crate::mir::nominal_factory::tests::fixture(false);
        let place = MirErasedPlace { local: MirErasedLocal(2), projection: vec![
            MirErasedProjection::NominalField { owner, field: FieldId(0) },
        ] };
        let function = program.backend_contract.erased.functions.get_mut(&key).unwrap();
        let body = function.body.as_mut().unwrap();
        body.locals[5] = MirErasedType::Parameter(0);
        body.blocks[0].statements[2].value = MirErasedRvalue::Use(MirErasedOperand::Move(place.clone()));
        body.blocks[0].statements[3].value = MirErasedRvalue::Literal(MirErasedLiteral::Unit);
        let paths = collect_erased_move_paths(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context).unwrap();
        program.backend_contract.erased.move_paths.insert(key.clone(), paths);
        (program, key, place)
    }

    #[test]
    fn partial_move_retains_siblings_but_never_reinitializes_the_whole_owner() {
        let (mut program, key, first) = fixture();
        assert_eq!(validate_erased_function(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context), Ok(()));
        let mut sibling = first.clone();
        let MirErasedProjection::NominalField { field, .. } = &mut sibling.projection[0] else { unreachable!() };
        *field = FieldId(1);
        for (place, expected) in [(sibling, Ok(())), (first, Err(MirErasedError::UninitializedOrMoved)),
            (MirErasedPlace::local(MirErasedLocal(2)), Err(MirErasedError::UninitializedOrMoved))] {
            let mut candidate = program.clone();
            let function = candidate.backend_contract.erased.functions.get_mut(&key).unwrap();
            let body = function.body.as_mut().unwrap();
            let ty = erased_place_type(&program.backend_contract.erased.functions[&key], &place, &program.backend_contract, &program.type_context).unwrap();
            body.locals.push(ty);
            body.blocks[0].statements.push(MirErasedAssignment {
                destination: MirErasedLocal((body.locals.len() - 1) as u32),
                value: MirErasedRvalue::Use(MirErasedOperand::Move(place)),
            });
            let paths = collect_erased_move_paths(&candidate.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context).unwrap();
            candidate.backend_contract.erased.move_paths.insert(key.clone(), paths);
            assert_eq!(validate_erased_function(&candidate.backend_contract.erased.functions[&key], &candidate.backend_contract, &candidate.type_context), expected);
        }
        // Layout evidence alone and changed cleanup partitions are not authority.
        program.backend_contract.erased.move_paths.get_mut(&key).unwrap().children[2].reverse();
        assert_eq!(validate_erased_function(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context), Err(MirErasedError::ProjectedMoveRequiresDropProof));
    }

    #[test]
    fn user_drop_or_indirect_storage_never_gain_a_partial_move_partition() {
        let (mut program, key, place) = fixture();
        let MirErasedProjection::NominalField { owner, .. } = place.projection[0] else { unreachable!() };
        program.backend_contract.nominal_drop_authority.insert(owner, MirNominalDropAuthority::UnsupportedUserDrop);
        assert_eq!(collect_erased_move_paths(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context), Err(MirErasedError::ProjectedMoveRequiresDropProof));
        program.backend_contract.nominal_drop_authority.insert(owner, MirNominalDropAuthority::FieldsOnly);
        let function = program.backend_contract.erased.functions.get_mut(&key).unwrap();
        let MirErasedRvalue::Use(MirErasedOperand::Move(place)) = &mut function.body.as_mut().unwrap().blocks[0].statements[2].value else { unreachable!() };
        place.projection.insert(0, MirErasedProjection::Deref);
        assert_eq!(collect_erased_move_paths(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context), Err(MirErasedError::ProjectedMoveRequiresDropProof));
    }

    #[test]
    fn move_partition_cannot_forge_cleanup_children_parentage_or_field_identity() {
        for control in 0..4 {
            let (mut program, key, _) = fixture();
            let paths = program.backend_contract.erased.move_paths.get_mut(&key).unwrap();
            match control {
                0 => { paths.children[2].pop(); },
                1 => { *paths.parents.last_mut().unwrap() = None; },
                2 => { paths.places.last_mut().unwrap().projection = vec![MirErasedProjection::Field(0)]; },
                _ => { paths.places.push(MirErasedPlace::local(MirErasedLocal(99))); },
            }
            assert_eq!(validate_erased_function(&program.backend_contract.erased.functions[&key], &program.backend_contract, &program.type_context), Err(MirErasedError::ProjectedMoveRequiresDropProof));
        }
    }
}

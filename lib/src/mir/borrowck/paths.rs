use std::collections::HashMap;

use crate::ids::{IdGen, Idx, MovePathId, PlacePathId};
use crate::mir::{
    Local, MirFunction, Operand, Place, Rvalue, StatementData, StatementKind, Terminator,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacePathData {
    pub id: PlacePathId,
    pub place: Place,
    pub parent: Option<PlacePathId>,
}

#[derive(Debug, Default)]
pub struct PlacePathTable {
    paths: Vec<PlacePathData>,
    by_place: HashMap<Place, PlacePathId>,
    ids: IdGen<PlacePathId>,
}

impl PlacePathTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn for_function(function: &MirFunction) -> Self {
        let mut table = Self::new();

        for local in 0..function.local_decls.len() {
            table.intern(root_place(Local(local)));
        }

        for block in &function.basic_blocks {
            for statement in &block.statements {
                collect_statement_places(statement, &mut table);
            }
            if let Some(terminator) = &block.terminator {
                collect_terminator_places(terminator, &mut table);
            }
        }

        table
    }

    pub fn intern(&mut self, place: Place) -> PlacePathId {
        if let Some(id) = self.by_place.get(&place) {
            return *id;
        }

        let parent = parent_place(&place).map(|parent| self.intern(parent));
        let id = self.ids.fresh();
        self.paths.push(PlacePathData {
            id,
            place: place.clone(),
            parent,
        });
        self.by_place.insert(place, id);
        id
    }

    pub fn get(&self, id: PlacePathId) -> Option<&PlacePathData> {
        self.paths.get(id.index())
    }

    pub fn place(&self, id: PlacePathId) -> Option<&Place> {
        self.get(id).map(|path| &path.place)
    }

    pub fn path_id(&self, place: &Place) -> Option<PlacePathId> {
        self.by_place.get(place).copied()
    }

    pub fn iter_ids(&self) -> impl Iterator<Item = PlacePathId> + '_ {
        self.paths.iter().map(|path| path.id)
    }

    pub fn paths_conflict(&self, a: PlacePathId, b: PlacePathId) -> bool {
        match (self.place(a), self.place(b)) {
            (Some(a), Some(b)) => crate::mir::borrowck::conflicts::places_conflict(a, b),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MovePathData {
    pub id: MovePathId,
    pub place_path: PlacePathId,
    pub parent: Option<MovePathId>,
    pub local: Local,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MovePathTable {
    paths: Vec<MovePathData>,
    by_place_path: HashMap<PlacePathId, MovePathId>,
}

impl MovePathTable {
    pub fn from_place_paths(place_paths: &PlacePathTable) -> Self {
        let mut table = Self::default();
        for place_path in place_paths.iter_ids() {
            table.intern_from_place_path(place_path, place_paths);
        }
        table
    }

    pub fn move_path_for_place(&self, place_path: PlacePathId) -> Option<MovePathId> {
        self.by_place_path.get(&place_path).copied()
    }

    pub fn place_path(&self, id: MovePathId) -> Option<PlacePathId> {
        self.paths.get(id.index()).map(|path| path.place_path)
    }

    pub fn iter_ids(&self) -> impl Iterator<Item = MovePathId> + '_ {
        self.paths.iter().map(|path| path.id)
    }

    pub fn local_roots(&self) -> HashMap<Local, MovePathId> {
        self.paths
            .iter()
            .filter_map(|path| path.parent.is_none().then_some((path.local, path.id)))
            .collect()
    }

    pub fn ancestors_inclusive(&self, move_path: MovePathId) -> Vec<MovePathId> {
        let mut ancestors = Vec::new();
        let mut current = Some(move_path);
        while let Some(id) = current {
            ancestors.push(id);
            current = self.paths.get(id.index()).and_then(|path| path.parent);
        }
        ancestors
    }

    pub fn descendants(&self, move_path: MovePathId) -> Vec<MovePathId> {
        self.paths
            .iter()
            .filter_map(|path| {
                (path.id != move_path && self.is_ancestor(move_path, path.id)).then_some(path.id)
            })
            .collect()
    }

    pub fn descendants_inclusive(&self, move_path: MovePathId) -> Vec<MovePathId> {
        self.paths
            .iter()
            .filter_map(|path| self.is_ancestor(move_path, path.id).then_some(path.id))
            .collect()
    }

    pub fn is_ancestor(&self, ancestor: MovePathId, child: MovePathId) -> bool {
        let mut current = Some(child);
        while let Some(id) = current {
            if id == ancestor {
                return true;
            }
            current = self.paths.get(id.index()).and_then(|path| path.parent);
        }
        false
    }

    fn intern_from_place_path(
        &mut self,
        place_path: PlacePathId,
        place_paths: &PlacePathTable,
    ) -> MovePathId {
        if let Some(id) = self.by_place_path.get(&place_path) {
            return *id;
        }

        let parent = place_paths
            .get(place_path)
            .and_then(|path| path.parent)
            .map(|parent| self.intern_from_place_path(parent, place_paths));
        let id = MovePathId(self.paths.len() as u32);
        let local = place_paths
            .place(place_path)
            .map(|place| place.local)
            .unwrap_or(Local(0));
        self.paths.push(MovePathData {
            id,
            place_path,
            parent,
            local,
        });
        self.by_place_path.insert(place_path, id);
        id
    }
}

fn collect_statement_places(statement: &StatementData, table: &mut PlacePathTable) {
    match &statement.kind {
        StatementKind::Assign(place, rvalue) => {
            table.intern(place.clone());
            collect_rvalue_places(rvalue, table);
        }
        StatementKind::Assert(assertion) => {
            for operand in &assertion.operands {
                collect_operand_place(operand, table);
            }
        }
        StatementKind::StorageLive(local) | StatementKind::StorageDead(local) => {
            table.intern(root_place(*local));
        }
    }
}

fn collect_rvalue_places(rvalue: &Rvalue, table: &mut PlacePathTable) {
    match rvalue {
        Rvalue::Use(operand) | Rvalue::Cast(operand, _) | Rvalue::UnaryOp(_, operand) => {
            collect_operand_place(operand, table);
        }
        Rvalue::Ref(_, place) | Rvalue::Discriminant(place) => {
            table.intern(place.clone());
        }
        Rvalue::BinaryOp(_, lhs, rhs) => {
            collect_operand_place(lhs, table);
            collect_operand_place(rhs, table);
        }
        Rvalue::Aggregate(_, operands) => {
            for operand in operands {
                collect_operand_place(operand, table);
            }
        }
        Rvalue::Closure(closure) => {
            for capture in &closure.captures {
                table.intern(capture.place());
            }
        }
    }
}

fn collect_terminator_places(terminator: &Terminator, table: &mut PlacePathTable) {
    match terminator {
        Terminator::Call {
            func,
            args,
            destination,
            ..
        } => {
            collect_operand_place(func, table);
            for arg in args {
                collect_operand_place(arg, table);
            }
            table.intern(destination.clone());
        }
        Terminator::SwitchInt { discr, .. } | Terminator::SwitchIntWithOrigin { discr, .. } => {
            collect_operand_place(discr, table);
        }
        Terminator::Drop { place, .. } | Terminator::DropWithOrigin { place, .. } => {
            table.intern(place.clone());
        }
        Terminator::Goto(_)
        | Terminator::GotoWithOrigin { .. }
        | Terminator::Return
        | Terminator::ReturnWithOrigin { .. } => {}
    }
}

fn collect_operand_place(operand: &Operand, table: &mut PlacePathTable) {
    match operand {
        Operand::Copy(place) | Operand::Move(place) => {
            table.intern(place.clone());
        }
        Operand::Constant(_) => {}
    }
}

fn root_place(local: Local) -> Place {
    Place {
        local,
        projection: Vec::new(),
    }
}

fn parent_place(place: &Place) -> Option<Place> {
    if place.projection.is_empty() {
        return None;
    }

    let mut parent = place.clone();
    parent.projection.pop();
    Some(parent)
}

#[cfg(test)]
mod tests;

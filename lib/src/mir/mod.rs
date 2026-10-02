//! Mid-level Intermediate Representation (MIR)
//!
//! MIR is a control-flow graph (CFG) representation of the program.
//! It is used for borrow checking, drop insertion (RAII), and optimizations.

pub mod agreement;
pub mod backend_contract;
pub mod borrowck;
pub mod builder;
pub mod dataflow;
pub mod erased;
pub mod identity;
pub mod object;
pub mod object_adapters;
pub use erased::*;
pub use object::*;
pub use object_adapters::*;

use crate::ids::{DefId, InstanceId, TypeId, VariantId};
use crate::lexer::Span;
use crate::type_context::TypeContext;
use std::collections::{BTreeMap, BTreeSet};

pub use backend_contract::{
    validate_backend_contract, validate_backend_contract_against_functions,
    validate_backend_contract_against_mir, MirArtifactExport, MirBackendContract,
    MirBackendContractError, MirCallableDecl, MirCallableKey, MirCallableKind,
    MirCallableSignature, MirLinkage, MirNominalLayout, MirParamAbi, MirPassMode, MirProjectionKey,
    MirReturnAbi,
};
pub use identity::{
    MirAggregateIdentity, MirAssertKind, MirCallable, MirClosureId, MirFieldIdentity,
    MirFunctionId, MirIntrinsicId, MirRuntimeHelper,
};

#[derive(Debug, Clone)]
pub struct MirProgram {
    pub functions: BTreeMap<MirFunctionId, MirFunction>,
    pub type_context: TypeContext,
    pub backend_contract: backend_contract::MirBackendContract,
}

impl MirProgram {
    pub fn functions(&self) -> impl Iterator<Item = (&MirFunctionId, &MirFunction)> {
        self.functions.iter()
    }

    pub fn function(&self, id: MirFunctionId) -> Option<&MirFunction> {
        self.functions.get(&id)
    }
}

#[derive(Debug, Clone, Default)]
pub struct MirInstanceBodies {
    bodies: BTreeMap<InstanceId, MirInstanceBody>,
    nested_functions: Vec<MirFunction>,
    runtime_instance_roots: BTreeSet<InstanceId>,
}

impl MirInstanceBodies {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, id: InstanceId, body: MirInstanceBody) {
        self.bodies.insert(id, body);
    }

    pub fn get(&self, id: InstanceId) -> Option<&MirInstanceBody> {
        self.bodies.get(&id)
    }

    pub fn contains_key(&self, id: InstanceId) -> bool {
        self.bodies.contains_key(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&InstanceId, &MirInstanceBody)> {
        self.bodies.iter()
    }

    pub fn retain<F>(&mut self, mut keep: F)
    where
        F: FnMut(InstanceId, &mut MirInstanceBody) -> bool,
    {
        self.bodies.retain(|id, body| keep(*id, body));
    }

    pub fn push_nested(&mut self, function: MirFunction) {
        self.nested_functions.push(function);
    }

    pub fn nested_functions(&self) -> &[MirFunction] {
        &self.nested_functions
    }

    pub fn retain_nested_functions<F>(&mut self, mut keep: F)
    where
        F: FnMut(&MirFunction) -> bool,
    {
        self.nested_functions.retain(|function| keep(function));
    }

    pub fn insert_runtime_instance_root(&mut self, id: InstanceId) {
        self.runtime_instance_roots.insert(id);
    }

    pub fn runtime_instance_roots(&self) -> impl Iterator<Item = InstanceId> + '_ {
        self.runtime_instance_roots.iter().copied()
    }
}

#[derive(Debug, Clone)]
pub struct MirInstanceBody {
    pub function: MirFunction,
    pub is_method: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MirBinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

impl From<crate::hir::BinOp> for MirBinOp {
    fn from(value: crate::hir::BinOp) -> Self {
        match value {
            crate::hir::BinOp::Add => Self::Add,
            crate::hir::BinOp::Sub => Self::Sub,
            crate::hir::BinOp::Mul => Self::Mul,
            crate::hir::BinOp::Div => Self::Div,
            crate::hir::BinOp::Mod => Self::Mod,
            crate::hir::BinOp::Eq => Self::Eq,
            crate::hir::BinOp::Ne => Self::Ne,
            crate::hir::BinOp::Lt => Self::Lt,
            crate::hir::BinOp::Le => Self::Le,
            crate::hir::BinOp::Gt => Self::Gt,
            crate::hir::BinOp::Ge => Self::Ge,
            crate::hir::BinOp::And => Self::And,
            crate::hir::BinOp::Or => Self::Or,
            crate::hir::BinOp::BitAnd => Self::BitAnd,
            crate::hir::BinOp::BitOr => Self::BitOr,
            crate::hir::BinOp::BitXor => Self::BitXor,
            crate::hir::BinOp::Shl => Self::Shl,
            crate::hir::BinOp::Shr => Self::Shr,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MirUnaryOp {
    Neg,
    Not,
    BitNot,
}

impl From<crate::hir::UnaryOp> for MirUnaryOp {
    fn from(value: crate::hir::UnaryOp) -> Self {
        match value {
            crate::hir::UnaryOp::Neg => Self::Neg,
            crate::hir::UnaryOp::Not => Self::Not,
            crate::hir::UnaryOp::BitNot => Self::BitNot,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MirClosureCaptureKind {
    ByValue,
    ByRef,
    ByMutRef,
}

impl From<crate::hir::HirClosureCaptureKind> for MirClosureCaptureKind {
    fn from(value: crate::hir::HirClosureCaptureKind) -> Self {
        match value {
            crate::hir::HirClosureCaptureKind::Move => Self::ByValue,
            crate::hir::HirClosureCaptureKind::SharedBorrow => Self::ByRef,
            crate::hir::HirClosureCaptureKind::MutableBorrow => Self::ByMutRef,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MirTraitMember {
    pub trait_id: DefId,
    pub name: String,
    pub member_id: DefId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirEnumVariantLayout {
    pub name: String,
    pub fields: MirVariantLayoutFields,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirVariantLayoutFields {
    Unit,
    Positional(Vec<TypeId>),
    Named(Vec<(String, TypeId)>),
}

#[derive(Debug, Clone)]
pub struct MirFunction {
    pub id: MirFunctionId,
    pub name: String,
    pub basic_blocks: Vec<BasicBlock>,
    pub local_decls: Vec<LocalDecl>,
    pub closure_captures: Vec<MirClosureCapture>,
    pub arg_count: usize,
    pub ret_type: TypeId,
    pub ownership: MirOwnershipMetadata,
}

impl MirFunction {
    pub fn terminator_origin(&self, terminator: &Terminator) -> MirOrigin {
        terminator.origin()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BorrowKind {
    Shared,
    Mutable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReferenceOrigin {
    Param(Local),
    Local(Local),
    Temporary(Local),
    Static,
    UnknownExternal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirSyntheticOrigin {
    Cleanup,
    Temporary,
    ClosureCapture,
    ControlFlow,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirOrigin {
    Source(Span),
    Synthetic {
        kind: MirSyntheticOrigin,
        origin: Option<Span>,
    },
}

impl MirOrigin {
    pub fn synthetic(kind: MirSyntheticOrigin, origin: Option<Span>) -> Self {
        Self::Synthetic { kind, origin }
    }

    pub fn source_span(&self) -> Option<&Span> {
        match self {
            Self::Source(span) => Some(span),
            Self::Synthetic { .. } => None,
        }
    }

    pub fn synthetic_origin(&self) -> Option<&Span> {
        match self {
            Self::Source(_) => None,
            Self::Synthetic { origin, .. } => origin.as_ref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MirOwnershipMetadata {
    pub reference_origins: Vec<(Local, ReferenceOrigin)>,
    pub temporary_locals: Vec<Local>,
    pub drop_obligations: Vec<MirDropObligation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropObligationKind {
    Direct,
    Structural,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirDropObligation {
    pub place: Place,
    pub ty: TypeId,
    pub kind: DropObligationKind,
}

#[derive(Debug, Clone)]
pub struct MirClosureCapture {
    pub name: String,
    pub local: Local,
    pub kind: MirClosureCaptureKind,
    pub span: Option<Span>,
}

#[derive(Debug, Clone)]
pub struct MirClosure {
    pub id: MirClosureId,
    pub display_name: String,
    pub captures: Vec<MirClosureCapture>,
}

#[derive(Debug, Clone)]
pub struct LocalDecl {
    pub ty: TypeId,
    pub mutability: Mutability,
    pub name: Option<String>,
    pub span: Option<Span>,
    pub source: LocalSource,
}

impl LocalDecl {
    pub fn origin(&self) -> MirOrigin {
        self.span.clone().map(MirOrigin::Source).unwrap_or_else(|| {
            MirOrigin::synthetic(
                if self.source.is_temporary() {
                    MirSyntheticOrigin::Temporary
                } else {
                    MirSyntheticOrigin::Unknown
                },
                None,
            )
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocalSource {
    ReturnPlace,
    Argument,
    ClosureCapture,
    UserBinding,
    Temporary,
}

impl LocalSource {
    pub fn is_temporary(self) -> bool {
        matches!(self, LocalSource::Temporary)
    }

    pub fn preserves_reference_liveness(self) -> bool {
        !self.is_temporary()
    }

    pub fn reports_immutable_mut_borrow(self) -> bool {
        !self.is_temporary()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BasicBlockId(pub usize);

#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub statements: Vec<StatementData>,
    pub terminator: Option<Terminator>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Local(pub usize);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Place {
    pub local: Local,
    pub projection: Vec<Projection>,
}

impl MirClosureCapture {
    pub fn place(&self) -> Place {
        Place {
            local: self.local,
            projection: vec![],
        }
    }

    pub fn origin(&self) -> MirOrigin {
        self.span
            .clone()
            .map(MirOrigin::Source)
            .unwrap_or_else(|| MirOrigin::synthetic(MirSyntheticOrigin::ClosureCapture, None))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Projection {
    Deref,
    Field {
        index: usize,
        identity: Option<MirFieldIdentity>,
    },
    Index(Local),
    Downcast(VariantId), // For enums
}

#[derive(Debug, Clone)]
pub enum Operand {
    Copy(Place),
    Move(Place),
    Constant(Constant),
}

#[derive(Debug, Clone)]
pub enum Constant {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Char(char),
    Callable(MirCallable),
    /// A call selector, legal only in a Call's `func`; args[0] supplies metadata.
    VirtualTarget(MirVirtualTarget),
    OwnedObjectCall(MirOwnedObjectKey),
    ErasedCall(MirErasedCall),
    TypeId(crate::ids::TypeId),
    Unit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mutability {
    Not,
    Mut,
}

#[derive(Debug, Clone)]
pub enum Rvalue {
    Use(Operand),
    Ref(Mutability, Place),
    Cast(Operand, TypeId),
    Object(Operand, MirObjectConversion),
    Closure(MirClosure),
    BinaryOp(MirBinOp, Operand, Operand),
    UnaryOp(MirUnaryOp, Operand),
    Discriminant(Place),
    Aggregate(AggregateKind, Vec<Operand>),
}

#[derive(Debug, Clone)]
pub struct MirAssert {
    pub kind: MirAssertKind,
    pub operands: Vec<Operand>,
}

#[derive(Debug, Clone)]
pub enum AggregateKind {
    Tuple,
    Array,
    Struct {
        id: crate::ids::DefId,
        display_name: String,
    },
    EnumVariant {
        enum_id: crate::ids::DefId,
        variant_id: crate::ids::VariantId,
        enum_name: String,
        variant_name: String,
    },
}

#[derive(Debug, Clone)]
pub struct StatementData {
    pub kind: StatementKind,
    pub span: Option<Span>,
    pub cleanup: bool,
}

impl StatementData {
    pub fn origin(&self) -> MirOrigin {
        self.span.clone().map(MirOrigin::Source).unwrap_or_else(|| {
            if self.cleanup {
                MirOrigin::synthetic(MirSyntheticOrigin::Cleanup, None)
            } else {
                MirOrigin::synthetic(MirSyntheticOrigin::Unknown, None)
            }
        })
    }

    pub fn new(kind: StatementKind, span: Option<Span>) -> Self {
        Self {
            kind,
            span,
            cleanup: false,
        }
    }

    pub fn cleanup(kind: StatementKind, span: Option<Span>) -> Self {
        Self {
            kind,
            span,
            cleanup: true,
        }
    }

    pub fn assign(place: Place, rvalue: Rvalue, span: Option<Span>) -> Self {
        Self::new(StatementKind::Assign(place, rvalue), span)
    }

    pub fn cleanup_assign(place: Place, rvalue: Rvalue, span: Option<Span>) -> Self {
        Self::cleanup(StatementKind::Assign(place, rvalue), span)
    }

    pub fn storage_live(local: Local, span: Option<Span>) -> Self {
        Self::new(StatementKind::StorageLive(local), span)
    }

    pub fn storage_dead(local: Local, span: Option<Span>) -> Self {
        Self::new(StatementKind::StorageDead(local), span)
    }

    pub fn assert(assertion: MirAssert, span: Option<Span>) -> Self {
        Self::new(StatementKind::Assert(assertion), span)
    }
}

#[derive(Debug, Clone)]
pub enum StatementKind {
    Assign(Place, Rvalue),
    Assert(MirAssert),
    StorageLive(Local),
    StorageDead(Local),
}

#[derive(Debug, Clone)]
pub enum Terminator {
    Unreachable {
        origin: MirOrigin,
    },
    Goto(BasicBlockId),
    GotoWithOrigin {
        target: BasicBlockId,
        origin: MirOrigin,
    },
    SwitchInt {
        discr: Operand,
        targets: Vec<(i64, BasicBlockId)>,
        otherwise: BasicBlockId,
    },
    SwitchIntWithOrigin {
        discr: Operand,
        targets: Vec<(i64, BasicBlockId)>,
        otherwise: BasicBlockId,
        origin: MirOrigin,
    },
    Return,
    ReturnWithOrigin {
        origin: MirOrigin,
    },
    Call {
        func: Operand,
        args: Vec<Operand>,
        destination: Place,
        target: BasicBlockId,
        span: Option<Span>,
    },
    Drop {
        place: Place,
        target: BasicBlockId,
    },
    DropWithOrigin {
        place: Place,
        target: BasicBlockId,
        origin: MirOrigin,
    },
}

impl Terminator {
    pub fn goto(target: BasicBlockId, origin: MirOrigin) -> Self {
        Self::GotoWithOrigin { target, origin }
    }

    pub fn switch_int(
        discr: Operand,
        targets: Vec<(i64, BasicBlockId)>,
        otherwise: BasicBlockId,
        origin: MirOrigin,
    ) -> Self {
        Self::SwitchIntWithOrigin {
            discr,
            targets,
            otherwise,
            origin,
        }
    }

    pub fn return_with_origin(origin: MirOrigin) -> Self {
        Self::ReturnWithOrigin { origin }
    }

    pub fn drop(place: Place, target: BasicBlockId, origin: MirOrigin) -> Self {
        Self::DropWithOrigin {
            place,
            target,
            origin,
        }
    }

    pub fn origin(&self) -> MirOrigin {
        match self {
            Self::Goto(_) | Self::SwitchInt { .. } | Self::Return | Self::Drop { .. } => {
                MirOrigin::synthetic(MirSyntheticOrigin::ControlFlow, None)
            }
            Self::GotoWithOrigin { origin, .. }
            | Self::Unreachable { origin }
            | Self::SwitchIntWithOrigin { origin, .. }
            | Self::ReturnWithOrigin { origin }
            | Self::DropWithOrigin { origin, .. } => origin.clone(),
            Self::Call { span, .. } => span
                .clone()
                .map(MirOrigin::Source)
                .unwrap_or_else(|| MirOrigin::synthetic(MirSyntheticOrigin::Unknown, None)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start: usize, end: usize) -> Span {
        Span::new("/virtual/main.rk".into(), start, end)
    }

    #[test]
    fn mir_origin_accessors_distinguish_source_and_synthetic_operations() {
        let source_span = span(2, 6);
        let source_statement = StatementData::assign(
            Place {
                local: Local(0),
                projection: vec![],
            },
            Rvalue::Use(Operand::Constant(Constant::Unit)),
            Some(source_span.clone()),
        );
        assert_eq!(
            source_statement.origin(),
            MirOrigin::Source(source_span.clone())
        );
        assert_eq!(
            StatementData::cleanup(StatementKind::StorageDead(Local(0)), None).origin(),
            MirOrigin::synthetic(MirSyntheticOrigin::Cleanup, None)
        );
        let linked = MirOrigin::synthetic(MirSyntheticOrigin::ControlFlow, Some(span(8, 9)));
        assert_eq!(linked.synthetic_origin(), Some(&span(8, 9)));

        let local = LocalDecl {
            ty: TypeContext::new().intern_type(&crate::types::Type::Unit),
            mutability: Mutability::Not,
            name: None,
            span: None,
            source: LocalSource::Temporary,
        };
        assert_eq!(
            local.origin(),
            MirOrigin::synthetic(MirSyntheticOrigin::Temporary, None)
        );
        let capture = MirClosureCapture {
            name: "value".to_string(),
            local: Local(0),
            kind: MirClosureCaptureKind::ByRef,
            span: Some(source_span),
        };
        assert!(matches!(capture.origin(), MirOrigin::Source(_)));

        let mut type_context = TypeContext::new();
        let unit = type_context.intern_type(&crate::types::Type::Unit);
        let function = MirFunction {
            id: MirFunctionId::Function(crate::ids::DefId::new(
                crate::ids::CrateId(0),
                crate::ids::LocalDefId(0),
            )),
            name: "main".to_string(),
            basic_blocks: vec![],
            local_decls: vec![
                local,
                LocalDecl {
                    ty: unit,
                    mutability: Mutability::Not,
                    name: None,
                    span: Some(span(20, 24)),
                    source: LocalSource::Temporary,
                },
            ],
            closure_captures: vec![],
            arg_count: 0,
            ret_type: unit,
            ownership: MirOwnershipMetadata::default(),
        };
        let call = Terminator::Call {
            func: Operand::Constant(Constant::Unit),
            args: vec![],
            destination: Place {
                local: Local(1),
                projection: vec![],
            },
            target: BasicBlockId(0),
            span: Some(span(2, 6)),
        };
        assert_eq!(
            function.terminator_origin(&call),
            MirOrigin::Source(span(2, 6))
        );

        let source = MirOrigin::Source(span(30, 35));
        assert_eq!(
            Terminator::switch_int(
                Operand::Constant(Constant::Bool(true)),
                vec![(1, BasicBlockId(1))],
                BasicBlockId(2),
                source.clone(),
            )
            .origin(),
            source
        );
        assert!(matches!(
            Terminator::goto(
                BasicBlockId(1),
                MirOrigin::synthetic(MirSyntheticOrigin::Cleanup, Some(span(40, 41))),
            )
            .origin(),
            MirOrigin::Synthetic {
                kind: MirSyntheticOrigin::Cleanup,
                origin: Some(_),
            }
        ));
    }
}

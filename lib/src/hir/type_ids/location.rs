use super::*;

impl HirTypeLocation {
    pub(crate) fn owner(&self) -> DefId {
        match self {
            Self::FunctionReturn { function }
            | Self::FunctionParam { function, .. }
            | Self::FunctionBoundArg { function, .. } => *function,
            Self::ExternReturn { extern_id } | Self::ExternParam { extern_id, .. } => *extern_id,
            Self::TraitSignatureReturn { trait_id, .. }
            | Self::TraitSignatureParam { trait_id, .. }
            | Self::TraitSignatureBoundArg { trait_id, .. } => *trait_id,
            Self::ImplReceiverArg { impl_id, .. }
            | Self::ImplTraitArg { impl_id, .. }
            | Self::ImplBoundArg { impl_id, .. }
            | Self::AssociatedTypeDef { impl_id, .. } => *impl_id,
            Self::TypeAliasBody { alias } => *alias,
            Self::ArrayRepeatLength { owner, .. }
            | Self::StructField { owner, .. }
            | Self::EnumVariantNamedField { owner, .. }
            | Self::EnumVariantPositionalField { owner, .. }
            | Self::PredicateSubject { owner, .. }
            | Self::PredicateArg { owner, .. }
            | Self::Block { owner, .. }
            | Self::LetStmt { owner, .. }
            | Self::Expr { owner, .. }
            | Self::ClosureCapture { owner, .. }
            | Self::ClosureParam { owner, .. }
            | Self::CastTarget { owner, .. }
            | Self::MethodCallTraitArg { owner, .. }
            | Self::MethodCallOwnerSubstitution { owner, .. }
            | Self::MethodCallMethodSubstitution { owner, .. }
            | Self::StaticCallOwner { owner, .. }
            | Self::StaticCallTraitArg { owner, .. }
            | Self::StaticCallOwnerSubstitution { owner, .. }
            | Self::StaticCallMethodSubstitution { owner, .. }
            | Self::TryOutput { owner, .. }
            | Self::TryResidual { owner, .. }
            | Self::TryReturn { owner, .. }
            | Self::TryBranchTraitArg { owner, .. }
            | Self::TryBranchOwnerSubstitution { owner, .. }
            | Self::TryBranchMethodSubstitution { owner, .. }
            | Self::TryFromResidualStaticCallOwner { owner, .. }
            | Self::TryFromResidualStaticCallTraitArg { owner, .. }
            | Self::TryFromResidualStaticCallOwnerSubstitution { owner, .. }
            | Self::TryFromResidualStaticCallMethodSubstitution { owner, .. }
            | Self::StructPatternArg { owner, .. } => *owner,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HirTypeLocation {
    ArrayRepeatLength {
        owner: DefId,
        path: Vec<usize>,
    },
    FunctionReturn {
        function: DefId,
    },
    FunctionParam {
        function: DefId,
        index: usize,
    },
    FunctionBoundArg {
        function: DefId,
        param: GenericParamId,
        bound_index: usize,
        arg_index: usize,
    },
    ExternReturn {
        extern_id: DefId,
    },
    ExternParam {
        extern_id: DefId,
        index: usize,
    },
    StructField {
        owner: DefId,
        field: FieldId,
    },
    EnumVariantNamedField {
        owner: DefId,
        variant: VariantId,
        field: FieldId,
    },
    EnumVariantPositionalField {
        owner: DefId,
        variant: VariantId,
        index: usize,
    },
    TraitSignatureReturn {
        trait_id: DefId,
        signature: DefId,
    },
    TraitSignatureParam {
        trait_id: DefId,
        signature: DefId,
        index: usize,
    },
    TraitSignatureBoundArg {
        trait_id: DefId,
        signature: DefId,
        param: GenericParamId,
        bound_index: usize,
        arg_index: usize,
    },
    ImplReceiverArg {
        impl_id: DefId,
        index: usize,
    },
    ImplTraitArg {
        impl_id: DefId,
        index: usize,
    },
    ImplBoundArg {
        impl_id: DefId,
        param: GenericParamId,
        bound_index: usize,
        arg_index: usize,
    },
    PredicateSubject {
        owner: DefId,
        index: usize,
    },
    PredicateArg {
        owner: DefId,
        predicate_index: usize,
        arg_index: usize,
    },
    AssociatedTypeDef {
        impl_id: DefId,
        assoc_type: AssocTypeId,
    },
    TypeAliasBody {
        alias: DefId,
    },
    Block {
        owner: DefId,
        path: Vec<usize>,
    },
    LetStmt {
        owner: DefId,
        path: Vec<usize>,
        name: String,
    },
    Expr {
        owner: DefId,
        path: Vec<usize>,
    },
    ClosureCapture {
        owner: DefId,
        path: Vec<usize>,
        name: String,
    },
    ClosureParam {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    CastTarget {
        owner: DefId,
        path: Vec<usize>,
    },
    MethodCallTraitArg {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    MethodCallOwnerSubstitution {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    MethodCallMethodSubstitution {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    StaticCallOwner {
        owner: DefId,
        path: Vec<usize>,
    },
    StaticCallTraitArg {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    StaticCallOwnerSubstitution {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    StaticCallMethodSubstitution {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    TryOutput {
        owner: DefId,
        path: Vec<usize>,
    },
    TryResidual {
        owner: DefId,
        path: Vec<usize>,
    },
    TryReturn {
        owner: DefId,
        path: Vec<usize>,
    },
    TryBranchTraitArg {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    TryBranchOwnerSubstitution {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    TryBranchMethodSubstitution {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    TryFromResidualStaticCallOwner {
        owner: DefId,
        path: Vec<usize>,
    },
    TryFromResidualStaticCallTraitArg {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    TryFromResidualStaticCallOwnerSubstitution {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    TryFromResidualStaticCallMethodSubstitution {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
    StructPatternArg {
        owner: DefId,
        path: Vec<usize>,
        index: usize,
    },
}

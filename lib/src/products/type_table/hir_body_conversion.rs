use super::collection_codec::{decode_patterns, encode_patterns};
use super::hir_schema::{
    SerializedHirBlock, SerializedHirCallTarget, SerializedHirClosureCapture, SerializedHirExpr,
    SerializedHirExprKind, SerializedHirMatchArm, SerializedHirMethodCallTarget,
    SerializedHirPattern, SerializedHirSelectedMethodTarget, SerializedHirSelectedTraitMember,
    SerializedHirStmt, SerializedHirStructLiteralField, SerializedHirStructPatternField,
    SerializedHirTypeBinding, SerializedHirVarRef, SerializedHirVarTarget,
};
use super::type_codec::{ProductTypeDecoder, ProductTypeEncoder};

impl SerializedHirBlock {
    pub(super) fn encode(
        value: &crate::hir::AcceptedHirBlock,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            stmts: value
                .stmts
                .iter()
                .map(|stmt| SerializedHirStmt::encode(stmt, encoder))
                .collect::<Result<_, _>>()?,
            ty: encoder.encode_type(&value.ty)?,
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::AcceptedHirBlock, String> {
        Ok(crate::hir::HirBlockFor::<crate::hir::AcceptedHir> {
            stmts: self
                .stmts
                .into_iter()
                .map(|stmt| stmt.decode(decoder))
                .collect::<Result<_, _>>()?,
            ty: decoder.decode_type(self.ty)?,
        })
    }
}

impl SerializedHirStmt {
    fn encode(
        value: &crate::hir::HirStmtFor<crate::hir::AcceptedHir>,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(match value {
            crate::hir::HirStmtFor::Let {
                name,
                local_id,
                ty,
                value,
                mutable,
            } => Self::Let {
                name: name.clone(),
                local_id: *local_id,
                ty: encoder.encode_type(ty)?,
                value: SerializedHirExpr::encode(value, encoder)?,
                mutable: *mutable,
            },
            crate::hir::HirStmtFor::Expr(expr) => {
                Self::Expr(SerializedHirExpr::encode(expr, encoder)?)
            }
            crate::hir::HirStmtFor::Return(expr) => Self::Return(
                expr.as_ref()
                    .map(|expr| SerializedHirExpr::encode(expr, encoder))
                    .transpose()?,
            ),
            crate::hir::HirStmtFor::Break(expr) => Self::Break(
                expr.as_ref()
                    .map(|expr| SerializedHirExpr::encode(expr, encoder))
                    .transpose()?,
            ),
            crate::hir::HirStmtFor::Continue => Self::Continue,
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirStmtFor<crate::hir::AcceptedHir>, String> {
        Ok(match self {
            Self::Let {
                name,
                local_id,
                ty,
                value,
                mutable,
            } => crate::hir::HirStmtFor::Let {
                name,
                local_id,
                ty: decoder.decode_type(ty)?,
                value: value.decode(decoder)?,
                mutable,
            },
            Self::Expr(expr) => crate::hir::HirStmtFor::Expr(expr.decode(decoder)?),
            Self::Return(expr) => {
                crate::hir::HirStmtFor::Return(expr.map(|expr| expr.decode(decoder)).transpose()?)
            }
            Self::Break(expr) => {
                crate::hir::HirStmtFor::Break(expr.map(|expr| expr.decode(decoder)).transpose()?)
            }
            Self::Continue => crate::hir::HirStmtFor::Continue,
        })
    }
}

impl SerializedHirExpr {
    pub(super) fn encode(
        value: &crate::hir::AcceptedHirExpr,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            kind: SerializedHirExprKind::encode(&value.kind, encoder)?,
            ty: encoder.encode_type(&value.ty)?,
            span: value.span.clone(),
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::AcceptedHirExpr, String> {
        Ok(crate::hir::HirExprFor::<crate::hir::AcceptedHir> {
            kind: self.kind.decode(decoder)?,
            ty: decoder.decode_type(self.ty)?,
            span: self.span,
        })
    }
}

impl SerializedHirClosureCapture {
    pub(super) fn encode(
        value: &crate::hir::HirClosureCapture,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            name: value.name.clone(),
            local_id: value.local_id,
            kind: value.kind,
            mutable: value.mutable,
            ty: encoder.encode_type(&value.ty)?,
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirClosureCapture, String> {
        Ok(crate::hir::HirClosureCapture {
            name: self.name,
            local_id: self.local_id,
            kind: self.kind,
            mutable: self.mutable,
            ty: decoder.decode_type(self.ty)?,
        })
    }
}

impl SerializedHirMethodCallTarget {
    pub(super) fn encode(
        value: &crate::hir::HirMethodCallTarget,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        let target = match &value.target {
            crate::hir::HirSelectedMethodTarget::ImplMethod {
                impl_id,
                method_id,
                selected_trait,
            } => SerializedHirSelectedMethodTarget::ImplMethod {
                impl_id: *impl_id,
                method_id: *method_id,
                selected_trait: selected_trait
                    .as_ref()
                    .map(|selected| {
                        Ok::<_, String>(SerializedHirSelectedTraitMember {
                            trait_id: selected.trait_id,
                            member_id: selected.member_id,
                            trait_args: encoder.encode_types(&selected.trait_args)?,
                        })
                    })
                    .transpose()?,
            },
            crate::hir::HirSelectedMethodTarget::TraitMethod {
                trait_id,
                member_id,
                trait_args,
                dispatch,
            } => SerializedHirSelectedMethodTarget::TraitMethod {
                trait_id: *trait_id,
                member_id: *member_id,
                trait_args: encoder.encode_types(trait_args)?,
                dispatch: *dispatch,
            },
        };
        let mut encode_bindings = |bindings: &[crate::hir::HirTypeBinding]| {
            bindings
                .iter()
                .map(|binding| {
                    Ok(SerializedHirTypeBinding {
                        param: binding.param,
                        ty: encoder.encode_type(&binding.ty)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        };
        Ok(Self {
            target,
            owner_substitution: encode_bindings(&value.owner_substitution)?,
            method_substitution: encode_bindings(&value.method_substitution)?,
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirMethodCallTarget, String> {
        let target = match self.target {
            SerializedHirSelectedMethodTarget::ImplMethod {
                impl_id,
                method_id,
                selected_trait,
            } => crate::hir::HirSelectedMethodTarget::ImplMethod {
                impl_id,
                method_id,
                selected_trait: selected_trait
                    .map(|selected| {
                        Ok::<_, String>(crate::hir::HirSelectedTraitMember {
                            trait_id: selected.trait_id,
                            member_id: selected.member_id,
                            trait_args: decoder.decode_types(&selected.trait_args)?,
                        })
                    })
                    .transpose()?,
            },
            SerializedHirSelectedMethodTarget::TraitMethod {
                trait_id,
                member_id,
                trait_args,
                dispatch,
            } => crate::hir::HirSelectedMethodTarget::TraitMethod {
                trait_id,
                member_id,
                trait_args: decoder.decode_types(&trait_args)?,
                dispatch,
            },
        };
        let mut decode_bindings = |bindings: Vec<SerializedHirTypeBinding>| {
            bindings
                .into_iter()
                .map(|binding| {
                    Ok(crate::hir::HirTypeBinding {
                        param: binding.param,
                        ty: decoder.decode_type(binding.ty)?,
                    })
                })
                .collect::<Result<Vec<_>, String>>()
        };
        Ok(crate::hir::HirMethodCallTarget {
            target,
            owner_substitution: decode_bindings(self.owner_substitution)?,
            method_substitution: decode_bindings(self.method_substitution)?,
        })
    }
}

impl SerializedHirVarRef {
    pub(super) fn from_hir(reference: &crate::hir::HirVarRef) -> Self {
        let target = match reference.target {
            crate::hir::HirVarTarget::Function(id) => SerializedHirVarTarget::Function(id),
            crate::hir::HirVarTarget::Extern(id) => SerializedHirVarTarget::Extern(id),
            crate::hir::HirVarTarget::Local(id) => SerializedHirVarTarget::Local(id),
            crate::hir::HirVarTarget::Instance(id) => {
                panic!("pre-mono products cannot serialize local instance variable {id:?}")
            }
        };
        Self {
            name: reference.name.clone(),
            target,
        }
    }

    pub(super) fn into_hir(self) -> crate::hir::HirVarRef {
        let target = match self.target {
            SerializedHirVarTarget::Function(id) => crate::hir::HirVarTarget::Function(id),
            SerializedHirVarTarget::Extern(id) => crate::hir::HirVarTarget::Extern(id),
            SerializedHirVarTarget::Local(id) => crate::hir::HirVarTarget::Local(id),
        };
        crate::hir::HirVarRef {
            name: self.name,
            target,
        }
    }
}

impl SerializedHirCallTarget {
    pub(super) fn encode(
        value: &crate::hir::HirCallTarget,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(match value {
            crate::hir::HirCallTarget::Function(id) => Self::Function(*id),
            crate::hir::HirCallTarget::Extern(id) => Self::Extern(*id),
            crate::hir::HirCallTarget::Instance(id) => {
                panic!("pre-mono products cannot serialize local instance target {id:?}")
            }
            crate::hir::HirCallTarget::Local(id) => Self::Local(*id),
            crate::hir::HirCallTarget::Intrinsic(name) => Self::Intrinsic(name.clone()),
            crate::hir::HirCallTarget::StaticMethod(target) => Self::StaticMethod {
                owner_ty: encoder.encode_type(&target.owner_ty)?,
                method: SerializedHirMethodCallTarget::encode(&target.method, encoder)?,
            },
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirCallTarget, String> {
        Ok(match self {
            Self::Function(id) => crate::hir::HirCallTarget::Function(id),
            Self::Extern(id) => crate::hir::HirCallTarget::Extern(id),
            Self::Local(id) => crate::hir::HirCallTarget::Local(id),
            Self::Intrinsic(name) => crate::hir::HirCallTarget::Intrinsic(name),
            Self::StaticMethod { owner_ty, method } => {
                crate::hir::HirCallTarget::StaticMethod(crate::hir::HirStaticMethodTarget {
                    owner_ty: decoder.decode_type(owner_ty)?,
                    method: method.decode(decoder)?,
                })
            }
        })
    }
}

impl SerializedHirStructLiteralField {
    pub(super) fn encode(
        value: &crate::hir::HirStructLiteralFieldFor<crate::hir::AcceptedHir>,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            name: value.name.clone(),
            value: SerializedHirExpr::encode(&value.value, encoder)?,
            field: value.field.clone(),
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirStructLiteralFieldFor<crate::hir::AcceptedHir>, String> {
        Ok(crate::hir::HirStructLiteralFieldFor {
            name: self.name,
            value: self.value.decode(decoder)?,
            field: self.field,
        })
    }
}

impl SerializedHirMatchArm {
    pub(super) fn encode(
        value: &crate::hir::HirMatchArmFor<crate::hir::AcceptedHir>,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            pattern: SerializedHirPattern::encode(&value.pattern, encoder)?,
            guard: value
                .guard
                .as_ref()
                .map(|guard| SerializedHirExpr::encode(guard, encoder))
                .transpose()?,
            body: SerializedHirBlock::encode(&value.body, encoder)?,
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirMatchArmFor<crate::hir::AcceptedHir>, String> {
        Ok(crate::hir::HirMatchArmFor {
            pattern: self.pattern.decode(decoder)?,
            guard: self.guard.map(|guard| guard.decode(decoder)).transpose()?,
            body: self.body.decode(decoder)?,
        })
    }
}

impl SerializedHirStructPatternField {
    fn encode(
        value: &crate::hir::HirStructPatternField,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(Self {
            name: value.name.clone(),
            field: value.field.clone(),
            pattern: SerializedHirPattern::encode(&value.pattern, encoder)?,
        })
    }

    fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirStructPatternField, String> {
        Ok(crate::hir::HirStructPatternField {
            name: self.name,
            field: self.field,
            pattern: self.pattern.decode(decoder)?,
        })
    }
}

impl SerializedHirPattern {
    pub(super) fn encode(
        value: &crate::hir::HirPattern,
        encoder: &mut ProductTypeEncoder,
    ) -> Result<Self, String> {
        Ok(match value {
            crate::hir::HirPattern::Wildcard => Self::Wildcard,
            crate::hir::HirPattern::Binding {
                name,
                local_id,
                mutable,
            } => Self::Binding {
                name: name.clone(),
                local_id: *local_id,
                mutable: *mutable,
            },
            crate::hir::HirPattern::Literal(value) => Self::Literal(value.clone()),
            crate::hir::HirPattern::Tuple(patterns) => {
                Self::Tuple(encode_patterns(patterns, encoder)?)
            }
            crate::hir::HirPattern::Struct(name, struct_id, type_args, fields) => Self::Struct(
                name.clone(),
                *struct_id,
                encoder.encode_types(type_args)?,
                fields
                    .iter()
                    .map(|field| SerializedHirStructPatternField::encode(field, encoder))
                    .collect::<Result<_, _>>()?,
            ),
            crate::hir::HirPattern::Enum(enum_name, variant_name, location, patterns) => {
                Self::Enum(
                    enum_name.clone(),
                    variant_name.clone(),
                    location.clone(),
                    encode_patterns(patterns, encoder)?,
                )
            }
            crate::hir::HirPattern::Or(patterns) => Self::Or(encode_patterns(patterns, encoder)?),
        })
    }

    pub(super) fn decode(
        self,
        decoder: &mut ProductTypeDecoder<'_>,
    ) -> Result<crate::hir::HirPattern, String> {
        Ok(match self {
            Self::Wildcard => crate::hir::HirPattern::Wildcard,
            Self::Binding {
                name,
                local_id,
                mutable,
            } => crate::hir::HirPattern::Binding {
                name,
                local_id,
                mutable,
            },
            Self::Literal(value) => crate::hir::HirPattern::Literal(value),
            Self::Tuple(patterns) => {
                crate::hir::HirPattern::Tuple(decode_patterns(patterns, decoder)?)
            }
            Self::Struct(name, struct_id, type_args, fields) => crate::hir::HirPattern::Struct(
                name,
                struct_id,
                decoder.decode_types(&type_args)?,
                fields
                    .into_iter()
                    .map(|field| field.decode(decoder))
                    .collect::<Result<_, _>>()?,
            ),
            Self::Enum(enum_name, variant_name, location, patterns) => {
                crate::hir::HirPattern::Enum(
                    enum_name,
                    variant_name,
                    location,
                    decode_patterns(patterns, decoder)?,
                )
            }
            Self::Or(patterns) => crate::hir::HirPattern::Or(decode_patterns(patterns, decoder)?),
        })
    }
}

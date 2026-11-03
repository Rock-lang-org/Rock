use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HirMethodCallTarget {
    pub target: HirSelectedMethodTarget,
    pub owner_substitution: Vec<HirTypeBinding>,
    pub method_substitution: Vec<HirTypeBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HirSelectedMethodTarget {
    ImplMethod {
        impl_id: DefId,
        method_id: DefId,
        selected_trait: Option<HirSelectedTraitMember>,
    },
    TraitMethod {
        trait_id: DefId,
        member_id: DefId,
        trait_args: Vec<Type>,
        dispatch: HirTraitDispatchKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HirSelectedTraitMember {
    pub trait_id: DefId,
    pub member_id: DefId,
    pub trait_args: Vec<Type>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HirTraitDispatchKind {
    TraitBound,
    CurrentTrait,
    UnresolvedGeneric,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HirTypeBinding {
    pub param: GenericParamId,
    pub ty: Type,
}

impl HirMethodCallTarget {
    pub fn impl_method(
        impl_id: DefId,
        method_id: DefId,
        selected_trait: Option<HirSelectedTraitMember>,
    ) -> Self {
        Self {
            target: HirSelectedMethodTarget::ImplMethod {
                impl_id,
                method_id,
                selected_trait,
            },
            owner_substitution: Vec::new(),
            method_substitution: Vec::new(),
        }
    }

    pub fn trait_method(
        trait_id: DefId,
        member_id: DefId,
        trait_args: Vec<Type>,
        dispatch: HirTraitDispatchKind,
    ) -> Self {
        Self {
            target: HirSelectedMethodTarget::TraitMethod {
                trait_id,
                member_id,
                trait_args,
                dispatch,
            },
            owner_substitution: Vec::new(),
            method_substitution: Vec::new(),
        }
    }

    pub fn impl_id(&self) -> Option<DefId> {
        match self.target {
            HirSelectedMethodTarget::ImplMethod { impl_id, .. } => Some(impl_id),
            HirSelectedMethodTarget::TraitMethod { .. } => None,
        }
    }

    pub fn trait_id(&self) -> Option<DefId> {
        match &self.target {
            HirSelectedMethodTarget::ImplMethod { selected_trait, .. } => {
                selected_trait.as_ref().map(|selected| selected.trait_id)
            }
            HirSelectedMethodTarget::TraitMethod { trait_id, .. } => Some(*trait_id),
        }
    }

    pub fn method_id(&self) -> Option<DefId> {
        match self.target {
            HirSelectedMethodTarget::ImplMethod { method_id, .. } => Some(method_id),
            HirSelectedMethodTarget::TraitMethod { member_id, .. } => Some(member_id),
        }
    }

    pub fn trait_args(&self) -> &[Type] {
        match &self.target {
            HirSelectedMethodTarget::ImplMethod { selected_trait, .. } => selected_trait
                .as_ref()
                .map(|selected| selected.trait_args.as_slice())
                .unwrap_or_default(),
            HirSelectedMethodTarget::TraitMethod { trait_args, .. } => trait_args,
        }
    }

    pub fn trait_args_mut(&mut self) -> Option<&mut Vec<Type>> {
        match &mut self.target {
            HirSelectedMethodTarget::ImplMethod { selected_trait, .. } => selected_trait
                .as_mut()
                .map(|selected| &mut selected.trait_args),
            HirSelectedMethodTarget::TraitMethod { trait_args, .. } => Some(trait_args),
        }
    }

    pub fn set_impl_method(
        &mut self,
        impl_id: DefId,
        method_id: DefId,
        selected_trait: Option<HirSelectedTraitMember>,
    ) {
        self.target = HirSelectedMethodTarget::ImplMethod {
            impl_id,
            method_id,
            selected_trait,
        };
    }

    pub fn for_each_type_mut(&mut self, mut visit: impl FnMut(&mut Type)) {
        if let Some(trait_args) = self.trait_args_mut() {
            for ty in trait_args {
                visit(ty);
            }
        }
        for binding in self
            .owner_substitution
            .iter_mut()
            .chain(self.method_substitution.iter_mut())
        {
            visit(&mut binding.ty);
        }
    }

    pub fn for_each_def_id_mut(&mut self, mut visit: impl FnMut(&mut DefId)) {
        match &mut self.target {
            HirSelectedMethodTarget::ImplMethod {
                impl_id,
                method_id,
                selected_trait,
            } => {
                visit(impl_id);
                visit(method_id);
                if let Some(selected_trait) = selected_trait {
                    visit(&mut selected_trait.trait_id);
                    visit(&mut selected_trait.member_id);
                }
            }
            HirSelectedMethodTarget::TraitMethod {
                trait_id,
                member_id,
                ..
            } => {
                visit(trait_id);
                visit(member_id);
            }
        }
        for binding in self
            .owner_substitution
            .iter_mut()
            .chain(self.method_substitution.iter_mut())
        {
            visit(&mut binding.param.owner);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HirStaticMethodTarget {
    pub owner_ty: Type,
    pub method: HirMethodCallTarget,
}

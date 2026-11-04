use super::*;

impl ArtifactCrateInterface {
    pub(crate) fn insert_function<P: crate::hir::HirPhase>(
        &mut self,
        canonical_name: String,
        function: crate::hir::HirFunctionFor<P>,
    ) {
        self.canonical_names.insert(function.id, canonical_name);
        self.functions
            .insert(function.id, ProductFunctionInterface::from(&function));
    }

    pub(crate) fn insert_struct(&mut self, canonical_name: String, strukt: HirStruct) {
        self.canonical_names.insert(strukt.id, canonical_name);
        self.structs
            .insert(strukt.id, ProductStructInterface::from(&strukt));
    }

    pub(crate) fn insert_trait<P: crate::hir::HirPhase>(
        &mut self,
        canonical_name: String,
        trt: crate::hir::HirTraitFor<P>,
    ) {
        self.canonical_names.insert(trt.id, canonical_name);
        self.traits
            .insert(trt.id, ProductTraitInterface::from(&trt));
    }

    pub(crate) fn insert_impl<P: crate::hir::HirPhase>(&mut self, imp: crate::hir::HirImplFor<P>) {
        let receiver_pattern = P::impl_receiver_pattern(&imp.receiver_pattern)
            .cloned()
            .unwrap_or_else(|| {
                let mut parts = Vec::new();
                P::visit_impl_receiver_types(&imp.receiver_pattern, &mut |ty| {
                    parts.push(ty.clone())
                });
                parts.into()
            });
        self.impls.insert(
            imp.id,
            ProductImplInterface {
                id: imp.id,
                owner: imp.owner,
                type_name: imp.type_name,
                type_generics: imp.type_generics,
                receiver_pattern,
                trait_name: imp.trait_name,
                trait_id: imp.trait_id,
                trait_generics: imp.trait_generics,
                trait_arg_types: imp.trait_arg_types,
                associated_types: imp
                    .associated_types
                    .into_iter()
                    .map(|assoc| crate::products::ProductAssociatedTypeInterface {
                        id: assoc.id,
                        name: assoc.name,
                        kind: assoc.kind,
                        ty: assoc.ty,
                    })
                    .collect(),
                bounds: imp.bounds,
                methods: imp
                    .methods
                    .iter()
                    .map(|(name, method)| (name.clone(), ProductFunctionInterface::from(method)))
                    .collect(),
            },
        );
    }

    pub(crate) fn function_by_canonical_name(
        &self,
        name: &str,
    ) -> Option<&ProductFunctionInterface> {
        self.id_for_canonical_name(name)
            .and_then(|id| self.functions.get(&id))
    }

    pub(crate) fn struct_by_canonical_name(&self, name: &str) -> Option<&ProductStructInterface> {
        self.id_for_canonical_name(name)
            .and_then(|id| self.structs.get(&id))
    }

    pub(crate) fn enum_by_canonical_name(&self, name: &str) -> Option<&ProductEnumInterface> {
        self.id_for_canonical_name(name)
            .and_then(|id| self.enums.get(&id))
    }

    pub(crate) fn trait_by_canonical_name(&self, name: &str) -> Option<&ProductTraitInterface> {
        self.id_for_canonical_name(name)
            .and_then(|id| self.traits.get(&id))
    }

    pub(crate) fn id_for_canonical_name(&self, name: &str) -> Option<DefId> {
        self.canonical_names
            .iter()
            .find_map(|(id, canonical)| (canonical == name).then_some(*id))
    }
}

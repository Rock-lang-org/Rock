//! Type lowering methods for Lowerer

use crate::ast;
use crate::hir::HirTrait;
use crate::type_lowering::{ResolvedNominalType, TypeLowerer, TypeLoweringContext};
use crate::types::Type;

use crate::lower::Lowerer;

impl TypeLoweringContext for Lowerer {
    fn nominal_type_params(&self, id: crate::ids::DefId) -> Vec<crate::types::GenericParamDecl> {
        self.items
            .structure(id)
            .map(|definition| definition.generic_params.clone())
            .or_else(|| {
                self.items
                    .enumeration(id)
                    .map(|definition| definition.generic_params.clone())
            })
            .unwrap_or_default()
    }
    fn sized_trait_id(&self) -> Option<crate::ids::DefId> {
        self.language_items.sized.as_ref().map(|item| item.trait_id)
    }
    fn push_type_error(&mut self, message: String, span: crate::lexer::Span) {
        self.diagnostics.push(message, span);
    }

    fn record_type_reference(
        &mut self,
        span: crate::lexer::Span,
        target: crate::source_map::SourceSymbol,
    ) {
        self.record_source_reference(span, target);
    }

    fn current_module_prefix(&self) -> Option<String> {
        Lowerer::current_module_prefix(self)
    }

    fn current_trait_name(&self) -> Option<String> {
        self.current_trait.clone()
    }

    fn resolve_nominal_type(&self, name: &str) -> Option<ResolvedNominalType> {
        crate::lower::resolution::LowerResolutionContext::new(self).resolve_nominal_type(name)
    }

    fn resolve_trait_type(&self, name: &str) -> Option<HirTrait> {
        crate::lower::resolution::LowerResolutionContext::new(self).resolve_trait_type(name)
    }

    fn existing_generic_type(&self, name: &str) -> Option<Type> {
        if let Some((_, binding)) = self.opened_witnesses.iter().rev().find(|(binder, _)| binder == name) {
            return Some(Type::Witness(binding.witness));
        }
        if let Some(parameter) = self
            .body_context
            .as_ref()
            .and_then(|context| context.generic_param_id(name))
        {
            return Some(Type::Generic(parameter));
        }
        let owner = self.current_generic_owner()?;
        let index = self
            .current_generic_params()
            .iter()
            .position(|parameter| parameter == name)?;
        Some(Type::Generic(crate::types::GenericParamId {
            owner,
            index: index as u32,
        }))
    }

    fn admit_object_type(
        &mut self,
        ty: Type,
        env: &crate::type_services::normalize::TypeNormalizationEnv,
        span: crate::lexer::Span,
    ) -> Type {
        match crate::traits::objects::admit_type_objects(&ty, self, env) {
            Ok(ty) => ty,
            Err(error) => {
                self.push_type_error(error.to_string(), span);
                Type::Error
            }
        }
    }

    fn qualify_object_type(
        &mut self,
        object: crate::types::ObjectType,
        bindings: Vec<crate::type_lowering::ObjectBindingSyntax>,
        _binders: Vec<Vec<crate::type_services::kind::Kind>>,
        span: crate::lexer::Span,
    ) -> Type {
        let traits = self
            .items
            .trait_defs()
            .map(|(id, definition)| (id, definition.clone()))
            .collect();
        match crate::type_lowering::resolve_object_bindings(object, bindings, &traits, span) {
            Ok(ty) => ty,
            Err((message, span)) => {
                self.push_type_error(message, span);
                Type::Error
            }
        }
    }

    fn resolve_type_alias(&self, name: &str) -> Option<crate::hir::HirTypeAlias> {
        let id =
            crate::lower::resolution::LowerResolutionContext::new(self).resolve_item_id(name)?;
        self.items.type_alias(id).cloned()
    }

    fn generic_type_for_name(&mut self, name: &str, span: crate::lexer::Span) -> Type {
        self.current_generic_type_for_name(name, span)
            .unwrap_or(Type::Error)
    }

    fn populate_type_normalization_env(
        &self,
        env: &mut crate::type_services::normalize::TypeNormalizationEnv,
    ) {
        for (_, structure) in self.items.structures() {
            env.register_constructor(
                structure.id,
                crate::types::NominalTypeKind::Struct,
                crate::type_lowering::constructor_kind(&structure.generic_params),
            );
            for param in &structure.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for (_, enumeration) in self.items.enumerations() {
            env.register_constructor(
                enumeration.id,
                crate::types::NominalTypeKind::Enum,
                crate::type_lowering::constructor_kind(&enumeration.generic_params),
            );
            for param in &enumeration.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for (_, trait_def) in self.items.trait_defs() {
            for param in &trait_def.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
            if let Some(target) = &trait_def.target {
                env.register_generic_kind(target.id, target.kind.clone());
            }
            for associated_type in &trait_def.associated_types {
                env.register_projection_kind(
                    crate::types::AssociatedTypeKey {
                        owner: trait_def.id,
                        assoc_type_id: associated_type.id,
                    },
                    associated_type.kind.clone(),
                );
            }
            for method in trait_def.methods.values() {
                for param in &method.generic_params {
                    env.register_generic_kind(param.id, param.kind.clone());
                }
            }
            for signature in trait_def.signatures.values() {
                for param in &signature.generic_params {
                    env.register_generic_kind(param.id, param.kind.clone());
                }
            }
        }
        for (_, function) in self.items.functions() {
            for param in &function.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for (_, signature) in self.items.function_sigs() {
            for param in &signature.generic_params {
                env.register_generic_kind(param.id, param.kind.clone());
            }
        }
        for (_, imp) in self.items.impl_defs() {
            for param in imp.type_generics.iter().chain(&imp.trait_generics) {
                env.register_generic_kind(param.id, param.kind.clone());
            }
            for method in imp.methods.values() {
                for param in &method.generic_params {
                    env.register_generic_kind(param.id, param.kind.clone());
                }
            }
        }
        crate::type_lowering::register_type_aliases(
            env,
            self.items.type_aliases().map(|(_, alias)| alias),
        );
    }
}

impl crate::traits::objects::ObjectTraitProvider for Lowerer {
    fn object_trait(
        &self,
        id: crate::ids::DefId,
    ) -> Option<crate::traits::objects::TraitTypeHeader> {
        self.trait_by_id(id).map(|definition| {
            crate::traits::objects::TraitTypeHeader::new(
                &definition.generic_params,
                definition.target.as_ref(),
                &definition.associated_types,
                &definition.predicates,
            )
        })
    }
}

impl Lowerer {
    pub(crate) fn refresh_inference_normalization_env(&mut self) {
        let mut env = crate::type_services::normalize::TypeNormalizationEnv::new();
        self.populate_type_normalization_env(&mut env);
        self.engine.set_normalization_env(env);
    }

    pub(crate) fn lower_parse_type(&mut self, parse_type: &ast::ParseType) -> Type {
        TypeLowerer::lower_parse_type(self, parse_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use crate::ast::{ParseType, ParseTypeInner};
    use crate::hir::{HirEnum, HirStruct, HirTrait};
    use crate::ids::{CrateId, DefId, LocalDefId};
    use crate::types::{GenericParamDecl, GenericParamId};

    fn named_type(name: &str) -> ParseType {
        ParseType::Type(ParseTypeInner {
            name: name.to_string(),
            generics: vec![],
            span: crate::lexer::Span::test(),
        })
    }

    fn def_id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    fn test_struct(id: DefId, name: &str) -> HirStruct {
        HirStruct {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        }
    }

    fn test_enum(id: DefId, name: &str) -> HirEnum {
        HirEnum {
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            variants: Vec::new(),
        }
    }

    fn test_trait(id: DefId, name: &str) -> HirTrait {
        HirTrait {
            target: None,
            predicates: Vec::new(),
            id,
            name: name.to_string(),
            generic_params: Vec::new(),
            associated_types: Vec::new(),
            methods: HashMap::new(),
            signatures: HashMap::new(),
        }
    }

    fn lowerer_with_nominals() -> Lowerer {
        let mut lowerer = Lowerer::new_for_test();
        lowerer.items.insert_structure(HirStruct {
            id: def_id(10),
            name: "Widget".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: def_id(10),
                    index: 0,
                },
                "T",
            )],
            fields: Vec::new(),
        });
        lowerer
            .resolver
            .item_paths
            .insert("Widget".to_string(), def_id(10));
        lowerer
            .resolver
            .item_names_by_id
            .insert(def_id(10), "Widget".to_string());
        lowerer.items.insert_enumeration(HirEnum {
            id: def_id(20),
            name: "Choice".to_string(),
            generic_params: vec![GenericParamDecl::type_param(
                GenericParamId {
                    owner: def_id(20),
                    index: 0,
                },
                "T",
            )],
            variants: Vec::new(),
        });
        lowerer
            .resolver
            .item_paths
            .insert("Choice".to_string(), def_id(20));
        lowerer
            .resolver
            .item_names_by_id
            .insert(def_id(20), "Choice".to_string());
        lowerer
    }

    #[test]
    fn lower_nominal_type_uses_decl_def_id() {
        let mut lowerer = lowerer_with_nominals();
        lowerer
            .resolver
            .item_names_by_id
            .insert(def_id(1), "other::Widget".to_string());
        lowerer
            .resolver
            .item_names_by_id
            .insert(def_id(2), "other::Choice".to_string());

        let struct_ty = lowerer.lower_parse_type(&ParseType::Type(ParseTypeInner {
            name: "Widget".to_string(),
            generics: vec![named_type("I64")],
            span: crate::lexer::Span::test(),
        }));
        let enum_ty = lowerer.lower_parse_type(&ParseType::Type(ParseTypeInner {
            name: "Choice".to_string(),
            generics: vec![named_type("Bool")],
            span: crate::lexer::Span::test(),
        }));

        assert_eq!(
            struct_ty,
            Type::Struct {
                id: def_id(10),
                args: vec![Type::I64],
            }
        );
        assert_eq!(
            enum_ty,
            Type::Enum {
                id: def_id(20),
                args: vec![Type::Bool],
            }
        );
        assert!(lowerer.source_map.references().iter().any(|reference| {
            reference.target == crate::source_map::SourceSymbol::Definition(def_id(10))
        }));
        assert!(lowerer.source_map.references().iter().any(|reference| {
            reference.target == crate::source_map::SourceSymbol::Definition(def_id(20))
        }));
    }

    #[test]
    fn lower_nominal_type_prefers_resolver_alias_id_over_suffix_match() {
        let canonical_id = def_id(40);
        let suffix_collision_id = def_id(41);
        let mut lowerer = Lowerer::new_for_test();
        lowerer.items.insert_structure(HirStruct {
            id: canonical_id,
            name: "dep::Widget".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        });
        lowerer.items.insert_structure(HirStruct {
            id: suffix_collision_id,
            name: "Widget".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        });
        lowerer.items.insert_structure(HirStruct {
            id: suffix_collision_id,
            name: "other::Widget".to_string(),
            generic_params: Vec::new(),
            fields: Vec::new(),
        });
        lowerer.resolver.insert_import_alias_with_name(
            "Widget".to_string(),
            "dep::Widget".to_string(),
            canonical_id,
        );

        let ty = lowerer.lower_parse_type(&named_type("Widget"));

        assert_eq!(
            ty,
            Type::Struct {
                id: canonical_id,
                args: Vec::new(),
            }
        );
    }

    #[test]
    fn lower_struct_type_prefers_module_local_alias_over_root_type() {
        let root_id = def_id(42);
        let module_id = def_id(43);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_structure(test_struct(root_id, "Thing"));
        lowerer
            .items
            .insert_structure(test_struct(module_id, "demo::helper::Thing"));
        lowerer
            .resolver
            .item_paths
            .insert("Thing".to_string(), root_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(root_id, "Thing".to_string());
        lowerer.resolver.insert_module_alias_with_name(
            "Thing".to_string(),
            "demo::helper::Thing".to_string(),
            module_id,
        );

        let ty = lowerer.lower_parse_type(&named_type("Thing"));

        assert_eq!(
            ty,
            Type::Struct {
                id: module_id,
                args: Vec::new(),
            }
        );
    }

    #[test]
    fn lower_enum_type_prefers_module_local_alias_over_root_type() {
        let root_id = def_id(44);
        let module_id = def_id(45);
        let mut lowerer = Lowerer::new_for_test();
        lowerer
            .items
            .insert_enumeration(test_enum(root_id, "Choice"));
        lowerer
            .items
            .insert_enumeration(test_enum(module_id, "demo::helper::Choice"));
        lowerer
            .resolver
            .item_paths
            .insert("Choice".to_string(), root_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(root_id, "Choice".to_string());
        lowerer.resolver.insert_module_alias_with_name(
            "Choice".to_string(),
            "demo::helper::Choice".to_string(),
            module_id,
        );

        let ty = lowerer.lower_parse_type(&named_type("Choice"));

        assert_eq!(
            ty,
            Type::Enum {
                id: module_id,
                args: Vec::new(),
            }
        );
    }

    #[test]
    fn resolve_trait_type_prefers_module_local_alias_over_root_trait() {
        let root_id = def_id(46);
        let module_id = def_id(47);
        let mut lowerer = Lowerer::new_for_test();
        lowerer.items.insert_trait_def(test_trait(root_id, "Show"));
        lowerer
            .items
            .insert_trait_def(test_trait(module_id, "demo::helper::Show"));
        lowerer
            .resolver
            .item_paths
            .insert("Show".to_string(), root_id);
        lowerer
            .resolver
            .item_names_by_id
            .insert(root_id, "Show".to_string());
        lowerer.resolver.insert_module_alias_with_name(
            "Show".to_string(),
            "demo::helper::Show".to_string(),
            module_id,
        );

        let trait_def = lowerer.resolve_trait_type("Show").unwrap();

        assert_eq!(trait_def.id, module_id);
    }

    #[test]
    fn test_lower_parse_bare_slice_type_reports_error() {
        let mut lowerer = Lowerer::new_for_test();

        let ty = lowerer.lower_parse_type(&ParseType::Slice(Box::new(named_type("I64"))));

        assert_eq!(ty, Type::Error);
        assert!(lowerer.errors().iter().any(|err| err
            .message
            .contains("bare slice type [T] must be written behind a reference")));
    }

    #[test]
    fn test_lower_parse_borrowed_slice_type_lowers_to_reference_to_slice_type() {
        let mut lowerer = Lowerer::new_for_test();

        let ty = lowerer.lower_parse_type(&ParseType::Reference {
            is_mut: false,
            pointee: Box::new(ParseType::Slice(Box::new(named_type("I64")))),
        });

        assert_eq!(
            ty,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Slice(Box::new(Type::I64))),
            }
        );
        assert!(lowerer.errors().is_empty());
    }

    #[test]
    fn test_lower_parse_raw_slice_pointer_lowers_to_pointer_to_slice_type() {
        let mut lowerer = Lowerer::new_for_test();

        let ty = lowerer.lower_parse_type(&ParseType::Pointer(Box::new(ParseType::Slice(
            Box::new(named_type("I64")),
        ))));

        assert_eq!(
            ty,
            Type::Pointer(Box::new(Type::Slice(Box::new(Type::I64))))
        );
        assert!(lowerer.errors().is_empty());
    }

    #[test]
    fn test_lower_parse_bare_str_reports_error() {
        let mut lowerer = Lowerer::new_for_test();

        let ty = lowerer.lower_parse_type(&named_type("Str"));

        assert_eq!(ty, Type::Error);
        assert!(lowerer.errors().iter().any(|err| err
            .message
            .contains("bare string slice type Str must be written behind a reference")));
    }

    #[test]
    fn test_lower_parse_borrowed_str_lowers_to_reference_to_str_type() {
        let mut lowerer = Lowerer::new_for_test();

        let ty = lowerer.lower_parse_type(&ParseType::Reference {
            is_mut: false,
            pointee: Box::new(named_type("Str")),
        });

        assert_eq!(
            ty,
            Type::Reference {
                mutable: false,
                inner: Box::new(Type::Str),
            }
        );
        assert!(lowerer.errors().is_empty());
    }

    #[test]
    fn test_lower_parse_raw_str_pointer_lowers_to_pointer_to_str_type() {
        let mut lowerer = Lowerer::new_for_test();

        let ty = lowerer.lower_parse_type(&ParseType::Pointer(Box::new(named_type("Str"))));

        assert_eq!(ty, Type::Pointer(Box::new(Type::Str)));
        assert!(lowerer.errors().is_empty());
    }

    #[test]
    fn test_lower_parse_mut_borrowed_str_lowers_to_mut_reference_to_str_type() {
        let mut lowerer = Lowerer::new_for_test();

        let ty = lowerer.lower_parse_type(&ParseType::Reference {
            is_mut: true,
            pointee: Box::new(named_type("Str")),
        });

        assert_eq!(
            ty,
            Type::Reference {
                mutable: true,
                inner: Box::new(Type::Str),
            }
        );
        assert!(lowerer.errors().is_empty());
    }

    #[test]
    fn test_lower_parse_fixed_array_type_preserves_length_without_error() {
        let mut lowerer = Lowerer::new_for_test();

        let ty = lowerer.lower_parse_type(&ParseType::Array {
            inner: Box::new(named_type("I64")),
            len: 4,
        });

        assert_eq!(ty, Type::Array(Box::new(Type::I64), 4));
        assert!(lowerer.errors().is_empty());
    }

    #[test]
    fn lower_parse_type_creates_implicit_generic_in_current_context() {
        let owner = def_id(60);
        let mut lowerer = Lowerer::new_for_test();
        lowerer.generic_context = Some(crate::lower::body_context::GenericLoweringContext::new(
            owner,
            Vec::new(),
        ));

        let ty = lowerer.lower_parse_type(&named_type("T"));

        assert_eq!(
            ty,
            Type::Generic(crate::types::GenericParamId { owner, index: 0 })
        );
        assert_eq!(lowerer.current_generic_params(), &["T".to_string()]);
        assert!(lowerer.errors().is_empty());
    }
}

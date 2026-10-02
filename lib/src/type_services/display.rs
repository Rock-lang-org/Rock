use std::collections::HashMap;
use std::fmt;

use crate::collect::resolver::ResolverTables;
use crate::ids::{DefId, Idx};
use crate::types::{AssociatedTypeKey, FunctionSafety, GenericParamId, Type};

pub struct TypeDisplay<'a> {
    ty: &'a Type,
}

pub fn display_type(ty: &Type) -> TypeDisplay<'_> {
    TypeDisplay { ty }
}

#[derive(Debug, Clone, Default)]
pub struct TypeDisplayContext {
    definitions: HashMap<DefId, String>,
    generics: HashMap<GenericParamId, String>,
    associated_types: HashMap<AssociatedTypeKey, String>,
}

impl TypeDisplayContext {
    pub fn from_resolver(resolver: &ResolverTables) -> Self {
        Self {
            definitions: resolver.item_names_by_id.clone(),
            generics: HashMap::new(),
            associated_types: HashMap::new(),
        }
    }

    pub fn extend_resolver(&mut self, resolver: &ResolverTables) {
        self.definitions.extend(
            resolver
                .item_names_by_id
                .iter()
                .map(|(id, name)| (*id, name.clone())),
        );
    }

    pub fn extend_hir_program<P: crate::hir::HirPhase>(
        &mut self,
        program: &crate::hir::HirProgramFor<P>,
    ) {
        for function in program.functions.values() {
            self.extend_generic_params(&function.generic_params);
        }
        for structure in program.structs.values() {
            self.extend_generic_params(&structure.generic_params);
        }
        for enumeration in program.enums.values() {
            self.extend_generic_params(&enumeration.generic_params);
        }
        for trait_def in program.traits.values() {
            self.extend_generic_params(&trait_def.generic_params);
            if let Some(target) = &trait_def.target {
                self.insert_generic_name(target.id, target.name.clone());
            }
            for associated in &trait_def.associated_types {
                self.insert_associated_name(
                    AssociatedTypeKey {
                        owner: trait_def.id,
                        assoc_type_id: associated.id,
                    },
                    associated.name.clone(),
                );
            }
            for method in trait_def.methods.values() {
                self.extend_generic_params(&method.generic_params);
            }
            for signature in trait_def.signatures.values() {
                self.extend_generic_params(&signature.generic_params);
            }
        }
        for impl_def in program.impls.values() {
            self.extend_generic_params(&impl_def.type_generics);
            self.extend_generic_params(&impl_def.trait_generics);
            for associated in &impl_def.associated_types {
                self.insert_associated_name(
                    AssociatedTypeKey {
                        owner: impl_def.id,
                        assoc_type_id: associated.id,
                    },
                    associated.name.clone(),
                );
            }
            for method in impl_def.methods.values() {
                self.extend_generic_params(&method.generic_params);
            }
        }
        for alias in program.type_aliases.values() {
            self.extend_generic_params(&alias.generic_params);
        }
    }

    fn extend_generic_params(&mut self, params: &[crate::types::GenericParamDecl]) {
        for generic in params {
            self.insert_generic_name(generic.id, generic.name.clone());
        }
    }

    pub fn insert_definition_name(&mut self, id: DefId, name: impl Into<String>) {
        self.definitions.insert(id, name.into());
    }

    pub fn insert_generic_name(&mut self, id: GenericParamId, name: impl Into<String>) {
        self.generics.insert(id, name.into());
    }

    pub fn insert_generic_names(&mut self, params: &[crate::types::GenericParamDecl]) {
        for param in params {
            self.insert_generic_name(param.id, param.name.clone());
        }
    }

    pub fn insert_associated_name(&mut self, id: AssociatedTypeKey, name: impl Into<String>) {
        self.associated_types.insert(id, name.into());
    }

    pub fn definition_name(&self, id: DefId) -> Option<&str> {
        self.definitions.get(&id).map(String::as_str)
    }
}

pub struct UserTypeDisplay<'a> {
    ty: &'a Type,
    context: &'a TypeDisplayContext,
}

pub fn display_type_with_context<'a>(
    ty: &'a Type,
    context: &'a TypeDisplayContext,
) -> UserTypeDisplay<'a> {
    UserTypeDisplay { ty, context }
}

fn write_object(
    object: &crate::types::ObjectType,
    context: Option<&TypeDisplayContext>,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    fn bound(
        bound: &crate::types::TraitBound,
        context: Option<&TypeDisplayContext>,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        if let Some(context) = context {
            write!(
                f,
                "{}",
                context
                    .definition_name(bound.trait_id)
                    .unwrap_or("<unknown trait>")
            )?;
        } else {
            write!(
                f,
                "trait#{}:{}",
                bound.trait_id.crate_id.0, bound.trait_id.local.0
            )?;
        }
        if !bound.type_args.is_empty() {
            write!(f, "<")?;
            for (index, arg) in bound.type_args.iter().enumerate() {
                if index > 0 {
                    write!(f, ", ")?;
                }
                if let Some(context) = context {
                    write_user_type(arg, context, f)?;
                } else {
                    write_type(arg, f)?;
                }
            }
            write!(f, ">")?;
        }
        Ok(())
    }
    bound(&object.principal, context, f)?;
    if object.guarantees.is_empty() && object.bindings.is_empty() {
        return Ok(());
    }
    write!(f, " {{ ")?;
    let mut first = true;
    for guarantee in &object.guarantees {
        if !first {
            write!(f, ", ")?;
        }
        first = false;
        bound(guarantee, context, f)?;
    }
    for binding in &object.bindings {
        if !first {
            write!(f, ", ")?;
        }
        first = false;
        bound(&binding.key.trait_ref, context, f)?;
        let key = AssociatedTypeKey {
            owner: binding.key.trait_ref.trait_id,
            assoc_type_id: binding.key.member,
        };
        if let Some(context) = context {
            write!(
                f,
                "::{} = ",
                context
                    .associated_types
                    .get(&key)
                    .map(String::as_str)
                    .unwrap_or("<unknown associated type>")
            )?;
            write_user_type(&binding.ty, context, f)?;
        } else {
            write!(f, "::assoc#{} = ", binding.key.member.0)?;
            write_type(&binding.ty, f)?;
        }
    }
    write!(f, " }}")
}

pub fn write_type(ty: &Type, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match ty {
        Type::Object(object) => write_object(object, None, f),
        Type::ObjectSelf { depth } => write!(f, "object-self^{depth}"),
        Type::Witness(witness) => write!(f, "witness({:?}:{:?})", witness.owner, witness.local),
        Type::I8 => write!(f, "I8"),
        Type::I16 => write!(f, "I16"),
        Type::I32 => write!(f, "I32"),
        Type::I64 => write!(f, "I64"),
        Type::U8 => write!(f, "U8"),
        Type::U16 => write!(f, "U16"),
        Type::U32 => write!(f, "U32"),
        Type::U64 => write!(f, "U64"),
        Type::F32 => write!(f, "F32"),
        Type::F64 => write!(f, "F64"),
        Type::Bool => write!(f, "Bool"),
        Type::Str => write!(f, "Str"),
        Type::Char => write!(f, "Char"),
        Type::Unit => write!(f, "()"),
        Type::Never => write!(f, "!"),
        Type::Slice(inner) => write!(f, "[{}]", display_type(inner)),
        Type::Array(inner, len) => write!(f, "[{}; {}]", display_type(inner), len),
        Type::Tuple(elems) => {
            write!(f, "(")?;
            for (index, elem) in elems.iter().enumerate() {
                if index > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{}", display_type(elem))?;
            }
            write!(f, ")")
        }
        Type::Function {
            params,
            ret,
            safety,
            ..
        } => {
            if matches!(safety, FunctionSafety::Unsafe) {
                write!(f, "unsafe ")?;
            }
            for (index, arg) in params.iter().enumerate() {
                if index > 0 {
                    write!(f, " -> ")?;
                }
                write!(f, "{}", display_type(arg))?;
            }
            if !params.is_empty() {
                write!(f, " -> ")?;
            }
            write!(f, "{}", display_type(ret))
        }
        Type::Struct { id, args } => {
            write!(f, "struct#{}::{}", id.crate_id.0, id.local.0)?;
            write_type_args(args, f)
        }
        Type::Enum { id, args } => {
            write!(f, "enum#{}::{}", id.crate_id.0, id.local.0)?;
            write_type_args(args, f)
        }
        Type::Reference { mutable, inner } => {
            if *mutable {
                write!(f, "&mut {}", display_type(inner))
            } else {
                write!(f, "&{}", display_type(inner))
            }
        }
        Type::Pointer(inner) => write!(f, "*{}", display_type(inner)),
        Type::TypeVar(id) => write!(f, "?T{}", id.raw()),
        Type::Generic(param) => write!(
            f,
            "generic#{}::{}.{}",
            param.owner.crate_id.0, param.owner.local.0, param.index
        ),
        Type::Projection {
            ty,
            trait_id,
            assoc_type,
            trait_args,
        } => {
            write!(
                f,
                "<{} as trait#{}::{}",
                display_type(ty),
                trait_id.crate_id.0,
                trait_id.local.0
            )?;
            write_type_args(trait_args, f)?;
            write!(
                f,
                ">::assoc#{}::{}.{}",
                assoc_type.owner.crate_id.0, assoc_type.owner.local.0, assoc_type.assoc_type_id.0
            )
        }
        Type::Constructor { id, flavor } => write!(
            f,
            "constructor[{flavor:?}]#{}::{}",
            id.crate_id.0, id.local.0
        ),
        Type::Apply { constructor, args } => {
            write!(f, "{}", display_type(constructor))?;
            write_type_args(args, f)
        }
        Type::Lambda { params, body } => {
            write!(f, "\\")?;
            for (index, kind) in params.iter().enumerate() {
                if index > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{kind}")?;
            }
            write!(f, " -> {}", display_type(body))
        }
        Type::BoundVar { depth, index, kind } => write!(f, "^{depth}.{index}:{kind}"),
        Type::Error => write!(f, "<error>"),
    }
}

fn write_type_args(args: &[Type], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if args.is_empty() {
        return Ok(());
    }

    write!(f, "<")?;
    for (index, arg) in args.iter().enumerate() {
        if index > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{}", display_type(arg))?;
    }
    write!(f, ">")
}

impl fmt::Display for TypeDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_type(self.ty, f)
    }
}

impl fmt::Display for UserTypeDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_user_type(self.ty, self.context, f)
    }
}

fn write_user_type(
    ty: &Type,
    context: &TypeDisplayContext,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    match ty {
        Type::Object(object) => write_object(object, Some(context), f),
        Type::ObjectSelf { .. } => write!(f, "<hidden Self>"),
        Type::Witness(_) => write!(f, "<opened witness>"),
        Type::I8 => write!(f, "I8"),
        Type::I16 => write!(f, "I16"),
        Type::I32 => write!(f, "I32"),
        Type::I64 => write!(f, "I64"),
        Type::U8 => write!(f, "U8"),
        Type::U16 => write!(f, "U16"),
        Type::U32 => write!(f, "U32"),
        Type::U64 => write!(f, "U64"),
        Type::F32 => write!(f, "F32"),
        Type::F64 => write!(f, "F64"),
        Type::Bool => write!(f, "Bool"),
        Type::Str => write!(f, "Str"),
        Type::Char => write!(f, "Char"),
        Type::Unit => write!(f, "()"),
        Type::Never => write!(f, "!"),
        Type::Slice(inner) => {
            write!(f, "[")?;
            write_user_type(inner, context, f)?;
            write!(f, "]")
        }
        Type::Array(inner, len) => {
            write!(f, "[")?;
            write_user_type(inner, context, f)?;
            write!(f, "; {len}]")
        }
        Type::Tuple(elements) => {
            write!(f, "(")?;
            write_user_type_list(elements, context, f)?;
            write!(f, ")")
        }
        Type::Function {
            params,
            ret,
            safety,
            ..
        } => {
            if matches!(safety, FunctionSafety::Unsafe) {
                write!(f, "unsafe ")?;
            }
            for param in params {
                write_user_type(param, context, f)?;
                write!(f, " -> ")?;
            }
            write_user_type(ret, context, f)
        }
        Type::Struct { id, args } | Type::Enum { id, args } => {
            write!(
                f,
                "{}",
                context.definition_name(*id).unwrap_or("<unknown type>")
            )?;
            write_user_type_args(args, context, f)
        }
        Type::Reference { mutable, inner } => {
            if *mutable {
                write!(f, "&mut ")?;
            } else {
                write!(f, "&")?;
            }
            write_user_type(inner, context, f)
        }
        Type::Pointer(inner) => {
            write!(f, "*")?;
            write_user_type(inner, context, f)
        }
        Type::TypeVar(_) | Type::Error => write!(f, "_"),
        Type::Generic(param) => write!(
            f,
            "{}",
            context
                .generics
                .get(param)
                .map(String::as_str)
                .unwrap_or("_")
        ),
        Type::Projection {
            ty,
            trait_id,
            assoc_type,
            trait_args,
        } => {
            write!(f, "<")?;
            write_user_type(ty, context, f)?;
            write!(
                f,
                " as {}",
                context
                    .definition_name(*trait_id)
                    .unwrap_or("<unknown trait>")
            )?;
            write_user_type_args(trait_args, context, f)?;
            write!(
                f,
                ">::{}",
                context
                    .associated_types
                    .get(assoc_type)
                    .map(String::as_str)
                    .unwrap_or("<unknown associated type>")
            )
        }
        Type::Constructor { id, .. } => {
            write!(
                f,
                "{}",
                context.definition_name(*id).unwrap_or("<unknown type>")
            )
        }
        Type::Apply { constructor, args } => {
            write_user_type(constructor, context, f)?;
            write_user_type_args(args, context, f)
        }
        Type::Lambda { params, body } => {
            write!(f, "\\")?;
            for (index, kind) in params.iter().enumerate() {
                if index > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{kind}")?;
            }
            write!(f, " -> ")?;
            write_user_type(body, context, f)
        }
        Type::BoundVar { .. } => write!(f, "_"),
    }
}

fn write_user_type_args(
    args: &[Type],
    context: &TypeDisplayContext,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    if args.is_empty() {
        return Ok(());
    }
    write!(f, " ")?;
    write_user_type_list(args, context, f)
}

fn write_user_type_list(
    types: &[Type],
    context: &TypeDisplayContext,
    f: &mut fmt::Formatter<'_>,
) -> fmt::Result {
    for (index, ty) in types.iter().enumerate() {
        if index > 0 {
            write!(f, ", ")?;
        }
        write_user_type(ty, context, f)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId, TypeVarId};
    use crate::types::{AssociatedTypeKey, GenericParamId};

    fn def_id(index: u32) -> DefId {
        DefId::new(CrateId(0), LocalDefId(index))
    }

    #[test]
    fn type_display_service_preserves_existing_output() {
        let generic = GenericParamId {
            owner: def_id(4),
            index: 1,
        };
        let projection = Type::Projection {
            ty: Box::new(Type::Generic(generic)),
            trait_id: def_id(5),
            assoc_type: AssociatedTypeKey {
                owner: def_id(5),
                assoc_type_id: AssocTypeId(2),
            },
            trait_args: vec![Type::I64, Type::Bool],
        };

        let cases = vec![
            (Type::I8, "I8"),
            (Type::U64, "U64"),
            (Type::F32, "F32"),
            (Type::Bool, "Bool"),
            (Type::Str, "Str"),
            (Type::Char, "Char"),
            (Type::Unit, "()"),
            (Type::Never, "!"),
            (Type::Slice(Box::new(Type::I64)), "[I64]"),
            (Type::Array(Box::new(Type::Bool), 4), "[Bool; 4]"),
            (Type::Tuple(vec![Type::I64, Type::Bool]), "(I64, Bool)"),
            (
                Type::function(vec![Type::I64, Type::Bool], Type::Unit),
                "I64 -> Bool -> ()",
            ),
            (
                Type::Struct {
                    id: def_id(1),
                    args: vec![Type::I64],
                },
                "struct#0::1<I64>",
            ),
            (
                Type::Enum {
                    id: def_id(2),
                    args: vec![Type::Bool],
                },
                "enum#0::2<Bool>",
            ),
            (
                Type::Reference {
                    mutable: false,
                    inner: Box::new(Type::Str),
                },
                "&Str",
            ),
            (
                Type::Reference {
                    mutable: true,
                    inner: Box::new(Type::I64),
                },
                "&mut I64",
            ),
            (Type::Pointer(Box::new(Type::I64)), "*I64"),
            (Type::TypeVar(TypeVarId(7)), "?T7"),
            (Type::Generic(generic), "generic#0::4.1"),
            (
                projection,
                "<generic#0::4.1 as trait#0::5<I64, Bool>>::assoc#0::5.2",
            ),
            (Type::Error, "<error>"),
        ];

        for (ty, expected) in cases {
            assert_eq!(display_type(&ty).to_string(), expected);
            assert_eq!(ty.to_string(), expected);
        }
    }

    #[test]
    fn user_type_display_uses_resolved_rock_names() {
        let owner = def_id(1);
        let result = def_id(2);
        let trait_id = def_id(5);
        let generic = GenericParamId { owner, index: 0 };
        let associated = AssociatedTypeKey {
            owner: trait_id,
            assoc_type_id: AssocTypeId(2),
        };
        let mut resolver = ResolverTables::default();
        resolver
            .item_names_by_id
            .insert(owner, "PtrBox".to_string());
        resolver
            .item_names_by_id
            .insert(result, "Result".to_string());
        resolver
            .item_names_by_id
            .insert(trait_id, "Iterator".to_string());
        let mut context = TypeDisplayContext::from_resolver(&resolver);
        context.insert_generic_name(generic, "T");
        context.insert_associated_name(associated, "Item");

        let ptr_box = Type::Struct {
            id: owner,
            args: vec![Type::Slice(Box::new(Type::I64))],
        };
        assert_eq!(
            display_type_with_context(&ptr_box, &context).to_string(),
            "PtrBox [I64]"
        );
        let nested = Type::Enum {
            id: result,
            args: vec![Type::Generic(generic), ptr_box],
        };
        assert_eq!(
            display_type_with_context(&nested, &context).to_string(),
            "Result T, PtrBox [I64]"
        );
        let projection = Type::Projection {
            ty: Box::new(Type::Generic(generic)),
            trait_id,
            assoc_type: associated,
            trait_args: Vec::new(),
        };
        assert_eq!(
            display_type_with_context(&projection, &context).to_string(),
            "<T as Iterator>::Item"
        );
    }

    #[test]
    fn user_type_display_never_exposes_unknown_semantic_ids() {
        let context = TypeDisplayContext::default();
        let unknown = Type::Struct {
            id: def_id(99),
            args: vec![Type::TypeVar(TypeVarId(7))],
        };
        let rendered = display_type_with_context(&unknown, &context).to_string();
        assert_eq!(rendered, "<unknown type> _");
        assert!(!rendered.contains("struct#"));
        assert!(!rendered.contains("?T"));
    }
}

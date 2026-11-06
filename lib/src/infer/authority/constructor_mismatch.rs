//! Explains why a built-in type cannot fill a type-constructor parameter.
//!
//! A parameter such as `value: F A` with `F _: Functor` only accepts a type
//! constructor applied to an argument (`Vec I64`, `Option T`). Built-in types
//! like arrays, slices, tuples and functions are not constructor applications,
//! so unification fails with an opaque constructor-inference error. This
//! names the call, the required traits and the types that implement them.

use crate::hir::{HirExpr, HirExprKind, HirVarTarget};
use crate::infer::UnifyError;
use crate::types::Type;

use super::MethodAuthorityContext;

impl MethodAuthorityContext<'_> {
    /// A clearer message for passing a built-in type where a type
    /// constructor application is expected, or `None` when that is not what
    /// `error` is about.
    pub(super) fn constructor_argument_mismatch_message(
        &self,
        callee: &HirExpr,
        index: usize,
        actual: &Type,
        error: &UnifyError,
    ) -> Option<String> {
        if !matches!(error, UnifyError::ConstructorInference { .. }) {
            return None;
        }
        let builtin = crate::infer::engine::non_constructor_type_description(actual)?;
        let actual = self.display_type(actual);

        let HirExprKind::ResolvedVar(reference) = &callee.kind else {
            return Some(format!(
                "this argument must be a type constructor applied to a type (like `F A`), \
                 but `{actual}` is {builtin}, which cannot be used that way"
            ));
        };
        let traits = match reference.target {
            HirVarTarget::Function(id) => self.constructor_parameter_traits(id, index),
            _ => Vec::new(),
        };

        let mut message = format!(
            "`{}` needs its argument to be a type constructor applied to a type (like `F A`)",
            short_name(&reference.name)
        );
        if !traits.is_empty() {
            let names = traits
                .iter()
                .map(|(name, _)| format!("`{}`", short_name(name)))
                .collect::<Vec<_>>()
                .join(" and ");
            message.push_str(&format!(" implementing {names}"));
        }
        message.push_str(&format!(
            ", but `{actual}` is {builtin}, which cannot be used that way"
        ));
        for (name, implementors) in traits {
            if !implementors.is_empty() {
                message.push_str(&format!(
                    "; `{}` is implemented for {}",
                    short_name(&name),
                    implementors.join(", ")
                ));
            }
        }
        Some(message)
    }

    /// The traits bounding the constructor of parameter `index` of
    /// function `id`, each with the types that implement it.
    fn constructor_parameter_traits(
        &self,
        id: crate::ids::DefId,
        index: usize,
    ) -> Vec<(String, Vec<String>)> {
        let Some(function) = self.functions.get(&id) else {
            return Vec::new();
        };
        let Some(Type::Apply { constructor, .. }) = function.params.get(index).map(|p| &p.ty)
        else {
            return Vec::new();
        };
        let Type::Generic(constructor) = constructor.as_ref() else {
            return Vec::new();
        };

        function
            .generic_bounds
            .get(constructor)
            .into_iter()
            .flatten()
            .filter_map(|bound| {
                let name = self.traits.get(&bound.trait_id)?.name.clone();
                let mut implementors = self
                    .impls
                    .values()
                    .filter(|imp| imp.trait_id == Some(bound.trait_id))
                    .map(|imp| imp.type_name.clone())
                    .collect::<Vec<_>>();
                implementors.sort();
                implementors.dedup();
                Some((name, implementors))
            })
            .collect()
    }
}

/// The last segment of a qualified name (`std::functional::functor::<&>`).
fn short_name(name: &str) -> &str {
    name.rsplit("::").next().unwrap_or(name)
}

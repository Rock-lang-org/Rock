use crate::ids::TypeVarId;
use crate::lexer::Span;
use crate::type_services::kind::Kind;
use crate::type_services::normalize::NormalizeError;
use crate::types::Type;

use super::InferenceEngine;

#[derive(Debug, Clone)]
pub enum UnifyError {
    Mismatch {
        expected: Type,
        found: Type,
    },
    InfiniteType {
        variable: TypeVarId,
        ty: Type,
    },
    KindMismatch {
        expected: Kind,
        found: Kind,
        ty: Option<Type>,
        context: &'static str,
    },
    ConstructorInference {
        pattern: Type,
        actual: Type,
    },
    AmbiguousConstructorHeads {
        left: Type,
        right: Type,
    },
    BinderEscape {
        ty: Type,
    },
    FunctionSafetyMismatch {
        expected: Type,
        found: Type,
    },
    ArityMismatch {
        kind: &'static str,
        expected: usize,
        found: usize,
        left: Type,
        right: Type,
    },
    MutabilityMismatch,
    Normalization {
        ty: Type,
        detail: NormalizeError,
    },
    AmbiguousType {
        kind: Kind,
    },
    NonCopyArrayRepeat {
        found: Type,
    },
    Message(String),
}

impl UnifyError {
    pub fn contains(&self, pattern: &str) -> bool {
        self.render_with(|ty| ty.to_string()).contains(pattern)
    }

    pub fn render(&self, engine: &InferenceEngine) -> String {
        self.render_with(|ty| engine.display_type(ty))
    }

    pub(crate) fn render_with(&self, display: impl Fn(&Type) -> String) -> String {
        match self {
            Self::Mismatch { expected, found } => {
                format!("Type mismatch: {} vs {}", display(expected), display(found))
            }
            Self::InfiniteType { ty, .. } => {
                format!("Infinite type: inferred type contains itself through {}", display(ty))
            }
            Self::KindMismatch {
                expected,
                found,
                ty,
                context,
            } => match ty {
                Some(ty) => format!(
                    "kind mismatch while {context} an inferred type to {}: {expected} vs {found}",
                    display(ty)
                ),
                None => format!("kind mismatch while {context}: {expected} vs {found}"),
            },
            Self::ConstructorInference { pattern, actual } => {
                match non_constructor_type_description(actual) {
                    Some(description) => format!(
                        "`{}` is {description}, not a type constructor applied to a type, \
                         so it cannot match `{}`",
                        display(actual),
                        display(pattern)
                    ),
                    None => format!(
                        "cannot infer a type constructor from {} = {}; add an explicit constructor section annotation",
                        display(pattern),
                        display(actual)
                    ),
                }
            }
            Self::AmbiguousConstructorHeads { left, right } => format!(
                "ambiguous constructor heads: {} vs {}",
                display(left),
                display(right)
            ),
            Self::BinderEscape { ty } => {
                format!("type lambda binder escapes while inferring {}", display(ty))
            }
            Self::FunctionSafetyMismatch { expected, found } => format!(
                "Unsafe function cannot be used as safe function: {} vs {}",
                display(expected),
                display(found)
            ),
            Self::ArityMismatch {
                kind,
                expected,
                found,
                left,
                right,
            } => match *kind {
                "tuple" => format!("Tuple length mismatch: {expected} vs {found}"),
                "function" => format!("Function argument count mismatch: {expected} vs {found}"),
                "generic" => format!(
                    "Generic argument count mismatch for {}: {expected} vs {found}",
                    display(left)
                ),
                "application" => format!(
                    "Type application arity mismatch: {} vs {}",
                    display(left),
                    display(right)
                ),
                _ => format!("Type mismatch: {} vs {}", display(left), display(right)),
            },
            Self::MutabilityMismatch => "Mutability mismatch on references".to_string(),
            Self::Normalization { ty, detail } => format!(
                "failed to normalize finalized type {}: {}",
                display(ty),
                display_normalize_error(detail, &display)
            ),
            Self::AmbiguousType { kind } if kind == &Kind::Type => {
                "ambiguous type: cannot determine type of expression; add a type annotation (e.g., `: I64`)".to_string()
            }
            Self::AmbiguousType { kind } => format!(
                "ambiguous type constructor: cannot determine a value of kind {kind}; add an explicit constructor section annotation"
            ),
            Self::NonCopyArrayRepeat { found } => format!(
                "array repeat initializer must be Copy, found '{}'",
                display(found)
            ),
            Self::Message(message) => message.clone(),
        }
    }
}

/// Names a built-in type that is not a type constructor application, such as
/// an array, slice, tuple or function; `None` for types that are or may be.
pub(crate) fn non_constructor_type_description(ty: &Type) -> Option<String> {
    Some(match ty {
        Type::Array(_, _) => "a built-in array type".to_string(),
        Type::Slice(_) => "a built-in slice type".to_string(),
        Type::Tuple(_) => "a tuple type".to_string(),
        Type::Function { .. } => "a function type".to_string(),
        Type::Reference { inner, .. } => {
            let inner = non_constructor_type_description(inner)
                .map(|description| description.trim_start_matches("a ").to_string())
                .unwrap_or_else(|| "type".to_string());
            format!("a reference to a {inner}")
        }
        Type::Pointer(_) => "a raw pointer type".to_string(),
        Type::Struct { .. }
        | Type::Enum { .. }
        | Type::Apply { .. }
        | Type::TypeVar(_)
        | Type::Generic(_)
        | Type::Projection { .. }
        | Type::Lambda { .. } => return None,
        _ => "a built-in type".to_string(),
    })
}

fn display_normalize_error(error: &NormalizeError, display: &impl Fn(&Type) -> String) -> String {
    match error {
        NormalizeError::AliasCycle(_) => "type alias cycle".to_string(),
        NormalizeError::DepthLimit { limit } => {
            format!("type normalization depth limit exceeded ({limit})")
        }
        NormalizeError::NodeLimit { limit } => {
            format!("type normalization node limit exceeded ({limit})")
        }
        NormalizeError::UnknownConstructor(_) => "unknown type constructor".to_string(),
        NormalizeError::KindMismatch { .. } => error.to_string(),
        NormalizeError::NotApplicable { kind } => {
            format!("type of kind {kind} is not applicable")
        }
        NormalizeError::NonCanonicalType { normalized } => format!(
            "type is not canonical; normalized form is {}",
            display(normalized)
        ),
    }
}

#[derive(Debug, Clone)]
pub struct InferenceError {
    pub error: UnifyError,
    pub span: Option<Span>,
}

impl InferenceError {
    pub fn contains(&self, pattern: &str) -> bool {
        self.error.contains(pattern)
    }
}

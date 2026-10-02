//! Capture-avoiding, simultaneous instantiation of declaration parameters.

use std::collections::HashMap;

use crate::type_services::kind::Kind;
use crate::type_services::visit::{
    try_fold_type, try_fold_type_children, TryTypeFolder, TypeVisitor,
};
use crate::types::{GenericParamId, Type};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubstitutionError {
    BinderDepthOverflow,
    InvalidBinderApplication,
}

impl std::fmt::Display for SubstitutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BinderDepthOverflow => write!(f, "binder depth overflow"),
            Self::InvalidBinderApplication => write!(f, "invalid binder application"),
        }
    }
}

struct Shift {
    lambda_shift: u32,
    object_shift: u32,
    lambda_cutoff: u32,
    object_cutoff: u32,
}

impl TryTypeFolder for Shift {
    type Error = SubstitutionError;

    fn try_enter_binders(&mut self, _: &[Kind]) -> Result<(), Self::Error> {
        self.lambda_cutoff = self
            .lambda_cutoff
            .checked_add(1)
            .ok_or(SubstitutionError::BinderDepthOverflow)?;
        Ok(())
    }
    fn try_exit_binders(&mut self) -> Result<(), Self::Error> {
        self.lambda_cutoff -= 1;
        Ok(())
    }
    fn try_enter_object(&mut self) -> Result<(), Self::Error> {
        self.object_cutoff = self
            .object_cutoff
            .checked_add(1)
            .ok_or(SubstitutionError::BinderDepthOverflow)?;
        Ok(())
    }
    fn try_exit_object(&mut self) -> Result<(), Self::Error> {
        self.object_cutoff -= 1;
        Ok(())
    }
    fn try_fold_type(&mut self, ty: Type) -> Result<Type, Self::Error> {
        match ty {
            Type::BoundVar { depth, index, kind } if depth >= self.lambda_cutoff => {
                Ok(Type::BoundVar {
                    depth: depth
                        .checked_add(self.lambda_shift)
                        .ok_or(SubstitutionError::BinderDepthOverflow)?,
                    index,
                    kind,
                })
            }
            Type::ObjectSelf { depth } if depth >= self.object_cutoff => Ok(Type::ObjectSelf {
                depth: depth
                    .checked_add(self.object_shift)
                    .ok_or(SubstitutionError::BinderDepthOverflow)?,
            }),
            other => try_fold_type_children(other, self),
        }
    }
}

pub fn shift_free_binders(
    ty: &Type,
    lambda_shift: u32,
    object_shift: u32,
) -> Result<Type, SubstitutionError> {
    try_fold_type(
        ty.clone(),
        &mut Shift {
            lambda_shift,
            object_shift,
            lambda_cutoff: 0,
            object_cutoff: 0,
        },
    )
}

pub fn substitute_lambda_group(
    ty: &Type,
    args: &[Type],
    consumed: usize,
    removes_group: bool,
) -> Result<Type, SubstitutionError> {
    if consumed != args.len() || u32::try_from(consumed).is_err() {
        return Err(SubstitutionError::InvalidBinderApplication);
    }
    struct Beta<'a> {
        args: &'a [Type],
        consumed: usize,
        removes_group: bool,
        lambdas: u32,
        objects: u32,
    }
    impl TryTypeFolder for Beta<'_> {
        type Error = SubstitutionError;
        fn try_enter_binders(&mut self, _: &[Kind]) -> Result<(), Self::Error> {
            self.lambdas = self
                .lambdas
                .checked_add(1)
                .ok_or(SubstitutionError::BinderDepthOverflow)?;
            Ok(())
        }
        fn try_exit_binders(&mut self) -> Result<(), Self::Error> {
            self.lambdas -= 1;
            Ok(())
        }
        fn try_enter_object(&mut self) -> Result<(), Self::Error> {
            self.objects = self
                .objects
                .checked_add(1)
                .ok_or(SubstitutionError::BinderDepthOverflow)?;
            Ok(())
        }
        fn try_exit_object(&mut self) -> Result<(), Self::Error> {
            self.objects -= 1;
            Ok(())
        }
        fn try_fold_type(&mut self, ty: Type) -> Result<Type, Self::Error> {
            match ty {
                Type::BoundVar { depth, index, kind } if depth == self.lambdas => {
                    if let Some(argument) = self.args.get(index as usize) {
                        let shift = self
                            .lambdas
                            .checked_add(u32::from(!self.removes_group))
                            .ok_or(SubstitutionError::BinderDepthOverflow)?;
                        shift_free_binders(argument, shift, self.objects)
                    } else {
                        Ok(Type::BoundVar {
                            depth,
                            index: index - self.consumed as u32,
                            kind,
                        })
                    }
                }
                Type::BoundVar { depth, index, kind }
                    if self.removes_group && depth > self.lambdas =>
                {
                    Ok(Type::BoundVar {
                        depth: depth - 1,
                        index,
                        kind,
                    })
                }
                other => try_fold_type_children(other, self),
            }
        }
    }
    try_fold_type(
        ty.clone(),
        &mut Beta {
            args,
            consumed,
            removes_group,
            lambdas: 0,
            objects: 0,
        },
    )
}

/// Unlike substitution-chain resolution, an argument is not interpreted again
/// as a parameter of the declaration into which it is being inserted.
pub fn instantiate(
    ty: &Type,
    arguments: &HashMap<GenericParamId, Type>,
) -> Result<Type, SubstitutionError> {
    struct Instantiate<'a> {
        arguments: &'a HashMap<GenericParamId, Type>,
        lambdas: u32,
        objects: u32,
    }
    impl TryTypeFolder for Instantiate<'_> {
        type Error = SubstitutionError;
        fn try_enter_binders(&mut self, _: &[Kind]) -> Result<(), Self::Error> {
            self.lambdas = self
                .lambdas
                .checked_add(1)
                .ok_or(SubstitutionError::BinderDepthOverflow)?;
            Ok(())
        }
        fn try_exit_binders(&mut self) -> Result<(), Self::Error> {
            self.lambdas -= 1;
            Ok(())
        }
        fn try_enter_object(&mut self) -> Result<(), Self::Error> {
            self.objects = self
                .objects
                .checked_add(1)
                .ok_or(SubstitutionError::BinderDepthOverflow)?;
            Ok(())
        }
        fn try_exit_object(&mut self) -> Result<(), Self::Error> {
            self.objects -= 1;
            Ok(())
        }
        fn try_fold_type(&mut self, ty: Type) -> Result<Type, Self::Error> {
            if let Type::Generic(param) = &ty {
                if let Some(argument) = self.arguments.get(param) {
                    return shift_free_binders(argument, self.lambdas, self.objects);
                }
            }
            try_fold_type_children(ty, self)
        }
    }
    try_fold_type(
        ty.clone(),
        &mut Instantiate {
            arguments,
            lambdas: 0,
            objects: 0,
        },
    )
}

pub fn has_free_object_self(ty: &Type) -> bool {
    has_free_object_self_at_depth(ty, 0)
}

/// Open one implicit object binder around a signature component. Nested object
/// binders keep their own Self; references to the opened binder are replaced by
/// the witness without capturing its free lambda/object variables.
pub fn instantiate_object_self(ty: &Type, witness: &Type) -> Result<Type, SubstitutionError> {
    struct Open<'a> { witness: &'a Type, lambdas: u32, objects: u32 }
    impl TryTypeFolder for Open<'_> {
        type Error = SubstitutionError;
        fn try_enter_binders(&mut self, _: &[Kind]) -> Result<(), Self::Error> {
            self.lambdas = self.lambdas.checked_add(1).ok_or(SubstitutionError::BinderDepthOverflow)?; Ok(())
        }
        fn try_exit_binders(&mut self) -> Result<(), Self::Error> { self.lambdas -= 1; Ok(()) }
        fn try_enter_object(&mut self) -> Result<(), Self::Error> {
            self.objects = self.objects.checked_add(1).ok_or(SubstitutionError::BinderDepthOverflow)?; Ok(())
        }
        fn try_exit_object(&mut self) -> Result<(), Self::Error> { self.objects -= 1; Ok(()) }
        fn try_fold_type(&mut self, ty: Type) -> Result<Type, Self::Error> {
            match ty {
                Type::ObjectSelf { depth } if depth == self.objects => shift_free_binders(self.witness, self.lambdas, self.objects),
                Type::ObjectSelf { depth } if depth > self.objects => Ok(Type::ObjectSelf { depth: depth - 1 }),
                other => try_fold_type_children(other, self),
            }
        }
    }
    try_fold_type(ty.clone(), &mut Open { witness, lambdas: 0, objects: 0 })
}

pub fn has_free_object_self_at_depth(ty: &Type, initial_depth: u32) -> bool {
    struct Scope {
        depth: u32,
        free: bool,
    }
    impl TypeVisitor for Scope {
        fn enter_object(&mut self) {
            self.depth += 1;
        }
        fn exit_object(&mut self) {
            self.depth -= 1;
        }
        fn visit_type(&mut self, ty: &Type) {
            if let Type::ObjectSelf { depth } = ty {
                self.free |= *depth >= self.depth;
            }
            crate::type_services::visit::visit_type_children(ty, self);
        }
    }
    let mut scope = Scope {
        depth: initial_depth,
        free: false,
    };
    scope.visit_type(ty);
    scope.free
}

#[cfg(test)]
mod tests {
    use crate::ids::{CrateId, DefId, LocalDefId};
    use crate::types::{ObjectType, TraitBound};

    use super::*;

    fn parameter(index: u32) -> GenericParamId {
        GenericParamId {
            owner: DefId::new(CrateId(0), LocalDefId(1)),
            index,
        }
    }
    fn object(argument: Type) -> Type {
        Type::Object(Box::new(ObjectType::new(TraitBound {
            trait_id: DefId::new(CrateId(0), LocalDefId(2)),
            type_args: vec![argument],
        })))
    }

    #[test]
    fn declaration_instantiation_does_not_reinterpret_actual_arguments() {
        let arguments = HashMap::from([
            (parameter(0), Type::Generic(parameter(1))),
            (parameter(1), Type::I64),
        ]);
        assert_eq!(
            instantiate(&Type::Generic(parameter(0)), &arguments).unwrap(),
            Type::Generic(parameter(1))
        );
    }

    #[test]
    fn insertion_under_lambda_and_object_preserves_both_binding_scopes() {
        let argument = Type::Tuple(vec![
            Type::BoundVar {
                depth: 0,
                index: 0,
                kind: Kind::Type,
            },
            Type::ObjectSelf { depth: 0 },
        ]);
        let template = Type::Lambda {
            params: vec![Kind::Type],
            body: Box::new(object(Type::Generic(parameter(0)))),
        };
        let actual = instantiate(&template, &HashMap::from([(parameter(0), argument)])).unwrap();
        let expected = Type::Lambda {
            params: vec![Kind::Type],
            body: Box::new(object(Type::Tuple(vec![
                Type::BoundVar {
                    depth: 1,
                    index: 0,
                    kind: Kind::Type,
                },
                Type::ObjectSelf { depth: 1 },
            ]))),
        };
        assert_eq!(actual, expected);
        assert!(has_free_object_self(&actual));
        assert!(!has_free_object_self(&object(actual)));
        let closed = object(Type::ObjectSelf { depth: 0 });
        assert_eq!(shift_free_binders(&closed, 0, 1).unwrap(), closed);
    }

    #[test]
    fn invalid_shift_depth_returns_an_error() {
        assert_eq!(
            shift_free_binders(&Type::ObjectSelf { depth: u32::MAX }, 0, 1),
            Err(SubstitutionError::BinderDepthOverflow)
        );
    }

    #[test]
    fn opening_an_object_preserves_inner_receivers_and_rebases_outer_receivers() {
        let signature = Type::Tuple(vec![Type::ObjectSelf { depth: 0 }, object(Type::ObjectSelf { depth: 1 }), object(Type::ObjectSelf { depth: 0 }), Type::ObjectSelf { depth: 1 }]);
        let expected = Type::Tuple(vec![Type::I64, object(Type::I64), object(Type::ObjectSelf { depth: 0 }), Type::ObjectSelf { depth: 0 }]);
        assert_eq!(instantiate_object_self(&signature, &Type::I64).unwrap(), expected);
    }

    #[test]
    fn partial_beta_reduction_does_not_capture_an_outer_lambda_variable() {
        let variable = |depth, index| Type::BoundVar {
            depth,
            index,
            kind: Kind::Type,
        };
        let body = Type::Tuple(vec![variable(0, 0), variable(0, 1)]);
        assert_eq!(
            substitute_lambda_group(&body, &[variable(0, 0)], 1, false).unwrap(),
            Type::Tuple(vec![variable(1, 0), variable(0, 0)])
        );
    }

    #[test]
    fn beta_reduction_shifts_a_hidden_receiver_under_a_new_object_binder() {
        let constructor = Type::Lambda {
            params: vec![Kind::Type],
            body: Box::new(object(Type::BoundVar {
                depth: 0,
                index: 0,
                kind: Kind::Type,
            })),
        };
        let applied = Type::Apply {
            constructor: Box::new(constructor),
            args: vec![Type::ObjectSelf { depth: 0 }],
        };
        let normalized = crate::type_services::normalize::TypeNormalizer::new(
            &crate::type_services::normalize::TypeNormalizationEnv::new(),
        )
        .normalize(&applied)
        .unwrap();
        assert_eq!(normalized, object(Type::ObjectSelf { depth: 1 }));
        assert!(!has_free_object_self(&object(normalized)));
    }
}

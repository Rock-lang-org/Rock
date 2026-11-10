use std::collections::HashMap;

use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId, TypeVarId};
use crate::types::{AssociatedTypeKey, CaptureKind, FunctionCapture, GenericParamId, Type};

use super::{
    fold_type, fold_type_children, fold_type_in_place, try_fold_type, try_fold_type_children,
    visit_type, visit_type_children, TryTypeFolder, TypeFolder, TypeVisitor,
};

fn def_id(index: u32) -> DefId {
    DefId::new(CrateId(0), LocalDefId(index))
}

fn generic(owner: u32, index: u32) -> GenericParamId {
    GenericParamId {
        owner: def_id(owner),
        index,
    }
}

fn nested_type() -> Type {
    Type::Projection {
        ty: Box::new(Type::Reference {
            mutable: false,
            inner: Box::new(Type::Struct {
                id: def_id(1),
                args: vec![Type::Function {
                    params: vec![
                        Type::Slice(Box::new(Type::TypeVar(TypeVarId(1)))),
                        Type::Tuple(vec![
                            Type::Generic(generic(10, 0)),
                            Type::Pointer(Box::new(Type::TypeVar(TypeVarId(2)))),
                        ]),
                    ],
                    ret: Box::new(Type::Array(
                        Box::new(Type::Enum {
                            id: def_id(2),
                            args: vec![Type::Generic(generic(11, 0))],
                        }),
                        crate::types::ArrayLen::Value(4),
                    )),
                    safety: crate::types::FunctionSafety::Safe,
                    callable_kind: crate::types::CallableKind::FnOnce,
                    captures: vec![FunctionCapture::new(
                        CaptureKind::Move,
                        Type::Reference {
                            mutable: true,
                            inner: Box::new(Type::TypeVar(TypeVarId(3))),
                        },
                    )],
                }],
            }),
        }),
        trait_id: def_id(3),
        assoc_type: AssociatedTypeKey {
            owner: def_id(4),
            assoc_type_id: AssocTypeId(0),
        },
        trait_args: vec![
            Type::Enum {
                id: def_id(5),
                args: vec![Type::TypeVar(TypeVarId(4))],
            },
            Type::Generic(generic(12, 0)),
        ],
    }
}

#[derive(Default)]
struct Recorder {
    nodes: usize,
    type_vars: Vec<TypeVarId>,
    generics: Vec<GenericParamId>,
}

impl TypeVisitor for Recorder {
    fn visit_type(&mut self, ty: &Type) {
        self.nodes += 1;
        match ty {
            Type::TypeVar(id) => self.type_vars.push(*id),
            Type::Generic(id) => self.generics.push(*id),
            _ => {}
        }
        visit_type_children(ty, self);
    }
}

#[test]
fn immutable_visitor_reaches_every_nested_type_once() {
    let mut recorder = Recorder::default();
    visit_type(&nested_type(), &mut recorder);

    // 18 types plus the array length, which is visited as a const-kinded node.
    assert_eq!(recorder.nodes, 19);
    assert_eq!(
        recorder.type_vars,
        vec![TypeVarId(1), TypeVarId(2), TypeVarId(3), TypeVarId(4)]
    );
    assert_eq!(
        recorder.generics,
        vec![generic(10, 0), generic(11, 0), generic(12, 0)]
    );
}

struct ReplaceTypeVars;

impl TypeFolder for ReplaceTypeVars {
    fn fold_type(&mut self, ty: Type) -> Type {
        match ty {
            Type::TypeVar(id) => Type::Generic(generic(20, id.0)),
            other => fold_type_children(other, self),
        }
    }
}

#[test]
fn mutable_folder_rewrites_every_nested_type_location() {
    let folded = fold_type(nested_type(), &mut ReplaceTypeVars);
    let mut recorder = Recorder::default();
    visit_type(&folded, &mut recorder);

    assert!(recorder.type_vars.is_empty());
    assert_eq!(
        recorder.generics,
        vec![
            generic(20, 1),
            generic(10, 0),
            generic(20, 2),
            generic(11, 0),
            generic(20, 3),
            generic(20, 4),
            generic(12, 0),
        ]
    );
}

struct RejectTypeVar(TypeVarId);

impl TryTypeFolder for RejectTypeVar {
    type Error = TypeVarId;

    fn try_fold_type(&mut self, ty: Type) -> Result<Type, Self::Error> {
        match ty {
            Type::TypeVar(id) if id == self.0 => Err(id),
            other => try_fold_type_children(other, self),
        }
    }
}

#[test]
fn fallible_folder_propagates_nested_errors() {
    assert_eq!(
        try_fold_type(nested_type(), &mut RejectTypeVar(TypeVarId(3))),
        Err(TypeVarId(3))
    );
}

struct PanicFolder;

impl TypeFolder for PanicFolder {
    fn fold_type(&mut self, ty: Type) -> Type {
        if matches!(ty, Type::TypeVar(TypeVarId(3))) {
            panic!("intentional folder panic");
        }
        fold_type_children(ty, self)
    }
}

#[test]
fn in_place_folder_preserves_original_type_when_folder_panics() {
    let mut ty = nested_type();
    let original = ty.clone();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        fold_type_in_place(&mut ty, &mut PanicFolder);
    }));

    assert!(result.is_err());
    assert_eq!(ty, original);
}

struct CountDefIds<'a>(&'a mut HashMap<DefId, usize>);

impl TypeFolder for CountDefIds<'_> {
    fn fold_type(&mut self, mut ty: Type) -> Type {
        let mut record = |id: DefId| {
            *self.0.entry(id).or_default() += 1;
        };
        match &mut ty {
            Type::Struct { id, .. } | Type::Enum { id, .. } => record(*id),
            Type::Projection {
                trait_id,
                assoc_type,
                ..
            } => {
                record(*trait_id);
                record(assoc_type.owner);
            }
            Type::Generic(param) => record(param.owner),
            _ => {}
        }
        fold_type_children(ty, self)
    }
}

#[test]
fn def_id_folder_reaches_every_nested_identity_once() {
    let mut counts = HashMap::new();
    fold_type(nested_type(), &mut CountDefIds(&mut counts));

    assert_eq!(counts.len(), 8);
    assert!(counts.values().all(|count| *count == 1));
}

#[test]
fn type_def_id_remap_preserves_original_type_when_callback_panics() {
    let mut ty = nested_type();
    let original = ty.clone();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ty.remap_def_ids(&mut |id| {
            if id == def_id(2) {
                panic!("intentional remap panic");
            }
            def_id(id.local.0 + 100)
        });
    }));

    assert!(result.is_err());
    assert_eq!(ty, original);
}

#[test]
fn visitor_sees_param_and_var_array_lengths_as_leaves() {
    let ty = Type::Tuple(vec![
        Type::Array(
            Box::new(Type::I64),
            crate::types::ArrayLen::Param(generic(7, 1)),
        ),
        Type::Array(Box::new(Type::I64), crate::types::ArrayLen::Var(TypeVarId(9))),
    ]);
    let mut recorder = Recorder::default();
    visit_type(&ty, &mut recorder);

    assert_eq!(recorder.generics, vec![generic(7, 1)]);
    assert_eq!(recorder.type_vars, vec![TypeVarId(9)]);
}

#[test]
fn generic_substitution_rewrites_array_lengths() {
    use crate::types::ArrayLen;

    let n = generic(7, 1);
    let ty = Type::Array(Box::new(Type::Generic(generic(7, 0))), ArrayLen::Param(n));
    let mut subst = HashMap::new();
    subst.insert(generic(7, 0), Type::I32);

    subst.insert(n, Type::ConstValue(3));
    assert_eq!(
        ty.substitute_generics(&subst),
        Type::Array(Box::new(Type::I32), ArrayLen::Value(3))
    );

    subst.insert(n, Type::Generic(generic(8, 0)));
    assert_eq!(
        ty.substitute_generics(&subst),
        Type::Array(Box::new(Type::I32), ArrayLen::Param(generic(8, 0)))
    );

    subst.insert(n, Type::TypeVar(TypeVarId(5)));
    assert_eq!(
        ty.substitute_generics(&subst),
        Type::Array(Box::new(Type::I32), ArrayLen::Var(TypeVarId(5)))
    );
}

#[test]
fn type_var_generalization_rewrites_array_length_vars() {
    use crate::types::ArrayLen;

    let ty = Type::Array(Box::new(Type::I64), ArrayLen::Var(TypeVarId(2)));
    let mut mapping = HashMap::new();
    mapping.insert(TypeVarId(2), generic(3, 0));
    assert_eq!(
        ty.replace_type_vars_to_generics(&mapping, &HashMap::new()),
        Type::Array(Box::new(Type::I64), ArrayLen::Param(generic(3, 0)))
    );
}

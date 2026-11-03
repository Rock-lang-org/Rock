//! Static owner signature relation matching.

use super::static_bound_paths::display_type_without_ids;
use crate::lower::Lowerer;
use crate::types::{GenericParamId, Type};
use std::collections::{HashMap, HashSet};

impl Lowerer {
    pub(super) fn infer_static_owner_signature_relation(
        receiver_pattern: &Type,
        signature: &Type,
        owner_params: &[GenericParamId],
        method_name: &str,
    ) -> Result<HashMap<GenericParamId, Type>, String> {
        let owner_params = owner_params.iter().copied().collect::<HashSet<_>>();
        let mut relation = HashMap::new();
        Self::collect_static_owner_signature_relations(
            receiver_pattern,
            signature,
            &owner_params,
            &mut relation,
            method_name,
        )?;
        Ok(relation)
    }

    fn collect_static_owner_signature_relations(
        receiver_pattern: &Type,
        signature: &Type,
        owner_params: &HashSet<GenericParamId>,
        relation: &mut HashMap<GenericParamId, Type>,
        method_name: &str,
    ) -> Result<(), String> {
        if let Some(candidate) =
            Self::static_owner_relation_for_matching_type(receiver_pattern, signature, owner_params)
        {
            for (owner_param, method_ty) in candidate {
                if let Some(previous) = relation.insert(owner_param, method_ty.clone()) {
                    if previous != method_ty {
                        return Err(format!(
                            "static method `{method_name}` has conflicting owner inference: {} vs {}",
                            display_type_without_ids(&previous),
                            display_type_without_ids(&method_ty)
                        ));
                    }
                }
            }
        }

        match signature {
            Type::Slice(inner) | Type::Pointer(inner) => {
                Self::collect_static_owner_signature_relations(
                    receiver_pattern,
                    inner,
                    owner_params,
                    relation,
                    method_name,
                )
            }
            Type::Array(inner, _) => Self::collect_static_owner_signature_relations(
                receiver_pattern,
                inner,
                owner_params,
                relation,
                method_name,
            ),
            Type::Tuple(elements) => {
                for element in elements {
                    Self::collect_static_owner_signature_relations(
                        receiver_pattern,
                        element,
                        owner_params,
                        relation,
                        method_name,
                    )?;
                }
                Ok(())
            }
            Type::Function {
                params,
                ret,
                captures,
                ..
            } => {
                for param in params {
                    Self::collect_static_owner_signature_relations(
                        receiver_pattern,
                        param,
                        owner_params,
                        relation,
                        method_name,
                    )?;
                }
                Self::collect_static_owner_signature_relations(
                    receiver_pattern,
                    ret,
                    owner_params,
                    relation,
                    method_name,
                )?;
                for capture in captures {
                    Self::collect_static_owner_signature_relations(
                        receiver_pattern,
                        &capture.ty,
                        owner_params,
                        relation,
                        method_name,
                    )?;
                }
                Ok(())
            }
            Type::Struct { args, .. } | Type::Enum { args, .. } => {
                for arg in args {
                    Self::collect_static_owner_signature_relations(
                        receiver_pattern,
                        arg,
                        owner_params,
                        relation,
                        method_name,
                    )?;
                }
                Ok(())
            }
            Type::Reference { inner, .. } => Self::collect_static_owner_signature_relations(
                receiver_pattern,
                inner,
                owner_params,
                relation,
                method_name,
            ),
            Type::Projection { ty, trait_args, .. } => {
                Self::collect_static_owner_signature_relations(
                    receiver_pattern,
                    ty,
                    owner_params,
                    relation,
                    method_name,
                )?;
                for trait_arg in trait_args {
                    Self::collect_static_owner_signature_relations(
                        receiver_pattern,
                        trait_arg,
                        owner_params,
                        relation,
                        method_name,
                    )?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn static_owner_relation_for_matching_type(
        receiver_pattern: &Type,
        signature_ty: &Type,
        owner_params: &HashSet<GenericParamId>,
    ) -> Option<HashMap<GenericParamId, Type>> {
        let mut relation = HashMap::new();
        Self::match_static_owner_pattern(
            receiver_pattern,
            signature_ty,
            owner_params,
            &mut relation,
        )
        .then_some(relation)
    }

    fn match_static_owner_pattern(
        receiver_pattern: &Type,
        signature_ty: &Type,
        owner_params: &HashSet<GenericParamId>,
        relation: &mut HashMap<GenericParamId, Type>,
    ) -> bool {
        match (receiver_pattern, signature_ty) {
            (Type::Generic(owner_param), signature_ty) if owner_params.contains(owner_param) => {
                relation
                    .insert(*owner_param, signature_ty.clone())
                    .is_none_or(|previous| previous == *signature_ty)
            }
            (Type::Slice(receiver), Type::Slice(signature))
            | (Type::Pointer(receiver), Type::Pointer(signature)) => {
                Self::match_static_owner_pattern(receiver, signature, owner_params, relation)
            }
            (Type::Array(receiver, receiver_len), Type::Array(signature, signature_len)) => {
                receiver_len == signature_len
                    && Self::match_static_owner_pattern(receiver, signature, owner_params, relation)
            }
            (Type::Tuple(receivers), Type::Tuple(signatures)) => {
                receivers.len() == signatures.len()
                    && receivers
                        .iter()
                        .zip(signatures)
                        .all(|(receiver, signature)| {
                            Self::match_static_owner_pattern(
                                receiver,
                                signature,
                                owner_params,
                                relation,
                            )
                        })
            }
            (
                Type::Function {
                    params: receiver_params,
                    ret: receiver_ret,
                    safety: receiver_safety,
                    ..
                },
                Type::Function {
                    params: signature_params,
                    ret: signature_ret,
                    safety: signature_safety,
                    ..
                },
            ) => {
                receiver_safety == signature_safety
                    && receiver_params.len() == signature_params.len()
                    && receiver_params
                        .iter()
                        .zip(signature_params)
                        .all(|(receiver, signature)| {
                            Self::match_static_owner_pattern(
                                receiver,
                                signature,
                                owner_params,
                                relation,
                            )
                        })
                    && Self::match_static_owner_pattern(
                        receiver_ret,
                        signature_ret,
                        owner_params,
                        relation,
                    )
            }
            (
                Type::Struct {
                    id: receiver_id,
                    args: receiver_args,
                },
                Type::Struct {
                    id: signature_id,
                    args: signature_args,
                },
            )
            | (
                Type::Enum {
                    id: receiver_id,
                    args: receiver_args,
                },
                Type::Enum {
                    id: signature_id,
                    args: signature_args,
                },
            ) => {
                receiver_id == signature_id
                    && receiver_args.len() == signature_args.len()
                    && receiver_args
                        .iter()
                        .zip(signature_args)
                        .all(|(receiver, signature)| {
                            Self::match_static_owner_pattern(
                                receiver,
                                signature,
                                owner_params,
                                relation,
                            )
                        })
            }
            (
                Type::Reference {
                    mutable: receiver_mutable,
                    inner: receiver_inner,
                },
                Type::Reference {
                    mutable: signature_mutable,
                    inner: signature_inner,
                },
            ) => {
                receiver_mutable == signature_mutable
                    && Self::match_static_owner_pattern(
                        receiver_inner,
                        signature_inner,
                        owner_params,
                        relation,
                    )
            }
            (
                Type::Projection {
                    ty: receiver_ty,
                    trait_id: receiver_trait,
                    assoc_type: receiver_assoc,
                    trait_args: receiver_args,
                },
                Type::Projection {
                    ty: signature_ty,
                    trait_id: signature_trait,
                    assoc_type: signature_assoc,
                    trait_args: signature_args,
                },
            ) => {
                receiver_trait == signature_trait
                    && receiver_assoc == signature_assoc
                    && receiver_args.len() == signature_args.len()
                    && Self::match_static_owner_pattern(
                        receiver_ty,
                        signature_ty,
                        owner_params,
                        relation,
                    )
                    && receiver_args
                        .iter()
                        .zip(signature_args)
                        .all(|(receiver, signature)| {
                            Self::match_static_owner_pattern(
                                receiver,
                                signature,
                                owner_params,
                                relation,
                            )
                        })
            }
            _ => receiver_pattern == signature_ty,
        }
    }
}

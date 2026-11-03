use crate::hir::{HirFunctionFor, HirFunctionSig, HirPhase, HirProgramFor, HirTraitFor};
use crate::ids::{AssocTypeId, DefId, VariantId};
use crate::language_items::{
    DropLanguageItems, FnLanguageItems, FnMutLanguageItems, FnOnceLanguageItems,
    IndexLanguageItems, IndexMutLanguageItems, RangeLanguageItems, SizedLanguageItems,
    TryLanguageItems,
};
use crate::types::ReceiverMode;
use crate::types::{GenericParamId, Type};

mod callables;
mod index;
mod range_sized_drop;
mod requirements;
mod try_protocol;

use callables::*;
use index::*;
use range_sized_drop::*;
use requirements::*;
use try_protocol::*;

pub(crate) fn validate_language_items<P: HirPhase>(program: &HirProgramFor<P>) -> Vec<String> {
    let mut errors = Vec::new();
    if let Some(items) = &program.language_items.fold {
        require_same_crate(
            "fold",
            [
                ("fold.trait", items.trait_id),
                ("fold.method", items.method_id),
            ],
            &mut errors,
        );
        if let Some(trait_def) = required_trait(program, items.trait_id, "fold.trait", &mut errors)
        {
            require_trait_generics(trait_def, "fold.trait", 1, &mut errors);
            if let Some(method) = required_trait_member(
                program,
                trait_def,
                items.trait_id,
                items.method_id,
                "fold.method",
                &mut errors,
            ) {
                require_receiver(
                    &method,
                    "fold.method",
                    Some(ReceiverMode::Move),
                    &mut errors,
                );
                require_receiver_type(
                    &method,
                    "fold.method",
                    self_ty(items.trait_id, 1),
                    &mut errors,
                );
                require_explicit_params(&method, "fold.method", 2, &mut errors);
                let params = method.params();
                match method.ret() {
                    Type::Enum { args, .. } if args.len() == 2 && params.get(2).is_some_and(|state| *state == &args[1]) => {},
                    _ => errors.push("fold.method must return a two-parameter control-flow enum preserving its accumulator type".to_string()),
                }
            }
        }
    }

    if let Some(items) = &program.language_items.sized {
        validate_sized(program, items, &mut errors);
    }
    if let Some(items) = &program.language_items.drop {
        validate_drop(program, items, &mut errors);
    }
    if let Some(items) = &program.language_items.index {
        validate_index(program, items, &mut errors);
    }
    if let Some(items) = &program.language_items.index_mut {
        validate_index_mut(program, items, &mut errors);
        validate_index_mut_pair(program.language_items.index.as_ref(), items, &mut errors);
    }
    if let Some(items) = &program.language_items.fn_once {
        validate_fn_once(program, items, &mut errors);
    }
    if let Some(items) = &program.language_items.fn_mut {
        validate_fn_mut(program, items, &mut errors);
    }
    if let Some(items) = &program.language_items.fn_trait {
        validate_fn(program, items, &mut errors);
    }
    if let Some(items) = &program.language_items.send {
        validate_marker_trait(program, items.trait_id, "send.trait", &mut errors);
    }
    if let Some(items) = &program.language_items.sync {
        validate_marker_trait(program, items.trait_id, "sync.trait", &mut errors);
    }
    if let Some(items) = &program.language_items.try_protocol {
        validate_try(program, items, &mut errors);
    }
    if let Some(items) = &program.language_items.range {
        validate_range(program, items, &mut errors);
    }

    errors
}

#[cfg(test)]
mod tests;

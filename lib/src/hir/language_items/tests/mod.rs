use crate::collect;
use crate::crate_system::CrateContext;
use crate::ids::DefId;
use crate::infer;
use crate::language_items::IndexLanguageItems;
use crate::lower;
use crate::types::ReceiverMode;
use crate::types::{GenericParamDecl, GenericParamId, Type};

use super::validate_language_items;

const VALID_DROP_SOURCE: &str = r#"lang drop
< trait Release
    lang method
    ~@release: ()
"#;

const VALID_SIZED_SOURCE: &str = r#"lang sized
< trait StaticLayout
"#;

const VALID_INDEX_SOURCE: &str = r#"lang index
< trait Lookup Key
    lang output
    type Value
    lang method
    @lookup: Key -> &Self::Value
"#;

const VALID_TRY_SOURCE: &str = r#"lang control_flow
< enum Flow B, C
    lang break
    Stop B
    lang continue
    Next C

lang try
< trait Carrier
    lang output
    type Value
    lang residual
    type Remainder
    lang branch
    ~@split: Flow Self::Remainder, Self::Value

lang from_residual
< trait Recover R
    lang method
    recover: R -> Self
"#;

fn resolved_language_item_program(source: &str) -> crate::infer::ResolvedHirProgram {
    let program = crate::parser::parse_string(source, &crate::Config::default())
        .expect("language-item source should parse");
    let context = CrateContext::new();
    let declarations = collect::collect(&program, &context, false, Some("core"))
        .expect("language-item source should collect");
    let partial =
        lower::program::lower_from_declarations(&program, declarations, &context, Some("core"))
            .expect("language-item source should lower");
    infer::finalize(partial).expect("language-item source should finalize")
}

fn default_method_seed() -> crate::hir::HirFunctionFor<crate::hir::AcceptedHir> {
    let hir = resolved_language_item_program(
        r#"lang drop
< trait Release
    lang method
    ~@release: ()
    ~@release = -> return
"#,
    );
    hir.program
        .program()
        .traits
        .values()
        .flat_map(|trait_def| trait_def.methods.values())
        .next()
        .expect("Drop default method")
        .clone()
}

fn signature_mut(
    program: &mut crate::hir::HirProgramFor<crate::hir::AcceptedHir>,
    method_id: DefId,
) -> &mut crate::hir::HirFunctionSig {
    program
        .traits
        .values_mut()
        .flat_map(|trait_def| trait_def.signatures.values_mut())
        .find(|signature| signature.id == method_id)
        .expect("marked method signature")
}

mod drop_sized;
mod index;
mod protocols;
mod try_protocol;

use super::*;

const VALID_CALLABLE_SOURCE: &str = r#"lang fn_once
< trait Once Args, Ret
    lang output
    type Output
    lang method
    ~@call_once: Args -> Ret

lang fn_mut
< trait Mutable Args, Ret
    lang output
    type Output
    lang method
    ^@call_mut: Args -> Ret

lang fn
< trait Shared Args, Ret
    lang output
    type Output
    lang method
    @call: Args -> Ret

lang send
< trait Transferable

lang sync
< trait Shareable

lang owned
< trait NoBorrows
"#;

#[test]
fn language_item_validation_accepts_renamed_finalized_protocols() {
    for source in [
        VALID_SIZED_SOURCE,
        VALID_DROP_SOURCE,
        VALID_INDEX_SOURCE,
        VALID_TRY_SOURCE,
        VALID_CALLABLE_SOURCE,
    ] {
        let hir = resolved_language_item_program(source);
        assert!(
            validate_language_items(hir.program.program()).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn language_item_validation_accepts_marked_default_method_by_id() {
    let hir = resolved_language_item_program(
        r#"lang drop
< trait Release
    lang method
    ~@release: ()
    ~@release = -> return
"#,
    );

    assert!(validate_language_items(hir.program.program()).is_empty());
    let items = hir
        .program
        .program()
        .language_items
        .drop
        .as_ref()
        .expect("Drop bundle");
    assert!(hir
        .program
        .program()
        .traits
        .get(&items.trait_id)
        .expect("Drop trait")
        .methods
        .values()
        .any(|method| method.id == items.method_id));
}

#[test]
fn language_item_validation_rejects_member_owned_by_another_trait() {
    let mut hir = resolved_language_item_program(
        r#"< trait Other
    @other: ()

lang drop
< trait Release
    lang method
    ~@release: ()
"#,
    );
    let program = hir.program.program_mut_for_test();
    let drop_id = program
        .language_items
        .drop
        .as_ref()
        .expect("Drop bundle")
        .trait_id;
    let other_method = program
        .traits
        .iter()
        .find(|(id, _)| **id != drop_id)
        .and_then(|(_, trait_def)| trait_def.signatures.values().next())
        .expect("other method")
        .id;
    program
        .language_items
        .drop
        .as_mut()
        .expect("Drop bundle")
        .method_id = other_method;

    assert!(validate_language_items(program)
        .iter()
        .any(|error| error.starts_with("drop.method") && error.contains("is owned by trait")));
}

#[test]
fn accepted_hir_revalidation_rejects_malformed_language_items() {
    let mut hir = resolved_language_item_program(VALID_DROP_SOURCE);
    let program = hir.program.program_mut_for_test();
    let method_id = program
        .language_items
        .drop
        .as_ref()
        .expect("Drop bundle")
        .method_id;
    signature_mut(program, method_id).ret = Type::I64;

    let errors = crate::hir::AcceptedHirProgram::revalidate_for_test(program.clone())
        .expect_err("accepted conversion must reject malformed Drop");
    assert_eq!(errors, vec!["drop.method must return ()".to_string()]);
}

#[test]
fn strict_finalization_reports_malformed_language_items() {
    let program = crate::parser::parse_string(VALID_DROP_SOURCE, &crate::Config::default())
        .expect("language-item source should parse");
    let context = CrateContext::new();
    let declarations = collect::collect(&program, &context, false, Some("core"))
        .expect("language-item source should collect");
    let mut partial =
        lower::program::lower_from_declarations(&program, declarations, &context, Some("core"))
            .expect("language-item source should lower");
    partial
        .language_items
        .drop
        .as_mut()
        .expect("Drop bundle")
        .method_id = DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(999));

    let errors =
        infer::finalize(partial).expect_err("strict finalization must report malformed providers");
    assert_eq!(
            errors.into_iter().map(|error| error.message).collect::<Vec<_>>(),
            vec!["drop.method DefId { crate_id: CrateId(0), local: LocalDefId(999) } is not declared by any trait".to_string()]
        );
}

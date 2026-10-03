//! Canonical package evidence shared by lowering and accepted-HIR validation.
use super::*;

pub(crate) fn prove_pack(
    context: &crate::infer::object_coercion::ObjectEvidenceContext<'_>,
    package: &crate::types::ExistentialType,
    witness: &Type,
    opened: Option<&crate::types::ObjectType>,
) -> Result<HirPackagePack, String> {
    if let Type::Generic(parameter) = witness {
        if context.bounds.relaxed_sized.contains(parameter) {
            return Err(
                "a Sized package requires sizedness evidence for its actual generic witness".into(),
            );
        }
    }
    let schema = crate::type_services::substitution::existential_object_schema(package)
        .map_err(|e| e.to_string())?;
    let source = Type::Reference {
        mutable: false,
        inner: Box::new(witness.clone()),
    };
    let target = Type::Reference {
        mutable: false,
        inner: Box::new(Type::Object(Box::new(schema))),
    };
    let evidence = if let Type::Witness(witness) = witness {
        context.prove_opened(
            &source,
            &target,
            *witness,
            opened.ok_or("package witness has no dominating bound evidence")?,
        )?
    } else {
        context.prove(&source, &target)?
    };
    let HirObjectEvidence::Concrete { source, views } = evidence else {
        return Err("package requires witness evidence, not an object upcast".into());
    };
    if &source != witness {
        return Err("package evidence has a different witness".into());
    }
    Ok(HirPackagePack {
        witness: witness.clone(),
        views,
    })
}

pub(crate) fn validate_pack<P: HirPhase>(
    program: &HirProgram,
    payload: &HirExprFor<P>,
    target: &Type,
    proof: &HirPackagePack,
    bounds: &HirGenericBounds,
    opened: Option<&crate::types::ObjectType>,
) -> Result<(), String> {
    let Type::Exists(package) = target else {
        return Err("package pack has a non-existential target".into());
    };
    if let HirExprKindFor::TupleLiteral(values) = &payload.kind {
        if payload.ty != Type::Tuple(values.iter().map(|value| value.ty.clone()).collect()) {
            return Err(
                "package tuple payload type disagrees with its actual initializer elements".into(),
            );
        }
    }
    let env = program.type_normalization_env();
    let admitted = crate::traits::objects::admit_type_objects(target, &program.traits, &env)
        .map_err(|e| e.to_string())?;
    if admitted != *target {
        return Err("package target is not canonical and complete".into());
    }
    crate::type_services::existential::validate_package_layouts(
        target,
        &program.structs,
        &program.enums,
        &env,
    )?;
    let witness =
        crate::type_services::existential::infer_existential_witness(package, &payload.ty, &env)?;
    if witness != proof.witness {
        return Err("package proof disagrees with its actual unique payload witness".into());
    }
    let context = crate::infer::object_coercion::ObjectEvidenceContext {
        traits: &program.traits,
        impls: &program.impls,
        structs: &program.structs,
        enums: &program.enums,
        language_items: &program.language_items,
        bounds,
    };
    let expected = prove_pack(&context, package, &witness, opened)?;
    super::validate_object_evidence(
        &HirObjectEvidence::Concrete {
            source: expected.witness,
            views: expected.views,
        },
        &HirObjectEvidence::Concrete {
            source: proof.witness.clone(),
            views: proof.views.clone(),
        },
    )
    .map_err(|message| format!("invalid existential package witness evidence: {message}"))
}

#[cfg(test)]
#[path = "packages_diagnostics_tests.rs"]
mod diagnostic_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn program() -> AcceptedHirProgram {
        let path = std::env::temp_dir().join("rock_package_hir/main.rk");
        crate::analyze(&crate::Config {
            entry_file: path.clone(),
            no_std: true,
            no_prelude: true,
            source_providers: vec![crate::SourceProvider::Virtual {
                path,
                text: r#"
trait Accept
    @accept: &Self -> ()
relay: (exists T where T: Accept in (&T, &T)) -> (exists U where U: Accept in (&U, &U))
relay = package ->
    open package as Hidden, pair
        a = pair.0
        b = pair.1
        a.accept b
        pack (exists Reclosed where Reclosed: Accept in (&Reclosed, &Reclosed)) (a, b)
main = !-> ()
"#
                .into(),
            }],
            ..Default::default()
        })
        .unwrap()
        .hir
        .program
    }

    #[test]
    fn accepted_package_open_repack_is_alpha_equivalent_and_preserves_shared_witness() {
        let program = program().into_program();
        let relay = program
            .functions
            .values()
            .find(|function| function.name == "relay")
            .unwrap();
        assert_eq!(relay.params[0].ty, relay.ret_type);
        let HirStmtFor::Expr(open) = &relay.body.stmts[0] else {
            panic!("open")
        };
        let HirExprKindFor::PackageOpen { binding, body, .. } = &open.kind else {
            panic!("package lane")
        };
        let HirStmtFor::Expr(pack) = body.stmts.last().unwrap() else {
            panic!("pack")
        };
        let HirExprKindFor::PackagePack { proof, .. } = &pack.kind else {
            panic!("pack lane")
        };
        assert_eq!(proof.witness, Type::Witness(binding.witness));
        assert_eq!(proof.views.len(), 1);
        assert!(AcceptedHirProgram::revalidate_for_test(program).is_ok());
    }

    #[test]
    fn accepted_package_rejects_forged_source_payload_and_proof_authority() {
        for forgery in 0..3 {
            let mut program = program();
            let relay = program
                .program_mut_for_test()
                .functions
                .values_mut()
                .find(|function| function.name == "relay")
                .unwrap();
            let HirStmtFor::Expr(open) = &mut relay.body.stmts[0] else {
                panic!("open")
            };
            let HirExprKindFor::PackageOpen { binding, body, .. } = &mut open.kind else {
                panic!("package lane")
            };
            match forgery {
                0 => binding.source_ty = Type::Unit,
                1 => binding.value.ty = Type::Tuple(vec![Type::I64, Type::I64]),
                _ => {
                    let HirStmtFor::Expr(pack) = body.stmts.last_mut().unwrap() else {
                        panic!("pack")
                    };
                    let HirExprKindFor::PackagePack { proof, .. } = &mut pack.kind else {
                        panic!("pack lane")
                    };
                    proof.views.clear();
                }
            }
            assert!(AcceptedHirProgram::revalidate_for_test(program.into_program()).is_err());
        }
    }
}

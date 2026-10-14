use std::sync::atomic::{AtomicU64, Ordering};

use inkwell::context::Context;

use crate::ids::{CrateId, DefId, LocalDefId};
use crate::mir::*;
use crate::types::Type;

fn array(name: &str, bytes: &[u8]) -> String {
    format!("static const unsigned char {name}[] = {{{}}};\n", bytes.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(","))
}
#[test]
fn selected_nominal_protocol_executes_exact_keys_templates_drops_and_rejection_controls() {
    for nested in [false, true] {
        let (mut program, _, owner) = crate::mir::nominal_factory::tests::fixture(nested);
        let context = Context::create();
        let cg = super::super::CodeGen::new(&context, "nominal_protocol");
        let target = cg.descriptor_target_key().unwrap();
        let tc = &mut program.type_context;
        let contract = &mut program.backend_contract;
        let later = tc.intern_type(&Type::Struct { id: DefId::new(CrateId(0), LocalDefId(300)), args: vec![] });
        let unit = tc.intern_type(&Type::Unit);
        let tag = tc.intern_type(&Type::I64);
        let callable = MirCallableKey::Extern(DefId::new(CrateId(0), LocalDefId(301)));
        contract.callables.insert(callable.clone(), MirCallableDecl {
            key: callable.clone(), source_def_id: Some(DefId::new(CrateId(0), LocalDefId(301))),
            kind: MirCallableKind::Extern { link_name: "later_drop".into(), variadic: false }, llvm_symbol: "later_drop".into(), linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[later], unit, MirPassMode::Pointer),
        });
        let later_drop = MirPayloadDropPlan { ty: later, user_drop: Some(callable), children: MirPayloadDropChildren::None };
        let tag_drop = payload_drop_plan(contract, tc, tag).unwrap();
        let fields = if nested {
            let pair = tc.intern_type(&Type::Tuple(vec![tc.type_for(later), tc.type_for(later)]));
            vec![MirPayloadDropPlan { ty: pair, user_drop: None, children: MirPayloadDropChildren::Fields(vec![later_drop.clone(), later_drop.clone()]) }, tag_drop.clone()]
        } else { vec![later_drop.clone(), later_drop.clone(), tag_drop.clone()] };
        let product = tc.intern_type(&Type::Struct { id: owner, args: vec![tc.type_for(later)] });
        let product_drop = MirPayloadDropPlan { ty: product, user_drop: None, children: MirPayloadDropChildren::Fields(fields) };
        let native = descriptor_identity::descriptor_identity(contract, tc, &product_drop).unwrap();
        let later_identity = descriptor_identity::descriptor_identity(contract, tc, &later_drop).unwrap();
        let tag_identity = descriptor_identity::descriptor_identity(contract, tc, &tag_drop).unwrap();
        let (template, _) = nominal_template(contract, tc, owner).unwrap();
        let empty_owner = DefId::new(CrateId(0), LocalDefId(302));
        contract.nominal_layouts.insert(empty_owner, MirNominalLayout::Struct {
            id: empty_owner, fields: vec![], generic_params: vec![crate::types::GenericParamId { owner: empty_owner, index: 0 }],
        });
        contract.nominal_field_ids.insert(empty_owner, vec![]);
        contract.nominal_drop_authority.insert(empty_owner, MirNominalDropAuthority::FieldsOnly);
        let empty_type = tc.intern_type(&Type::Struct { id: empty_owner, args: vec![tc.type_for(later)] });
        let empty_drop = MirPayloadDropPlan { ty: empty_type, user_drop: None, children: MirPayloadDropChildren::None };
        let empty_identity = descriptor_identity::descriptor_identity(contract, tc, &empty_drop).unwrap();
        let (empty_template, _) = nominal_template(contract, tc, empty_owner).unwrap();
        let selected_owner = DefId::new(CrateId(0), LocalDefId(303));
        contract.nominal_layouts.insert(selected_owner, MirNominalLayout::Struct {
            id: selected_owner, fields: vec![], generic_params: vec![crate::types::GenericParamId { owner: selected_owner, index: 0 }],
        });
        contract.nominal_field_ids.insert(selected_owner, vec![]);
        contract.nominal_drop_authority.insert(selected_owner, MirNominalDropAuthority::FieldsOnly);
        let (selected_template, _) = nominal_template(contract, tc, selected_owner).unwrap();
        let selection = MirNominalDropSelection {
            implementation: selected_owner, member: selected_owner,
            entry: MirErasedKey { definition: DefId::new(CrateId(0), LocalDefId(304)), static_args: vec![] },
            arguments: vec![MirErasedType::Parameter(0)],
        };
        let selected_drop_key = nominal_factory::nominal_drop_template_key(contract, tc, &selection).unwrap();
        let mut duplicate = template.clone();
        // RN01 header = magic + framed origin + argument count + field count.
        let origin_len = u64::from_le_bytes(template[4..12].try_into().unwrap()) as usize;
        let field_start = 28 + origin_len;
        let expression_len = u64::from_le_bytes(template[field_start + 8..field_start + 16].try_into().unwrap()) as usize;
        let second = field_start + 16 + expression_len;
        let first_id = template[field_start..field_start + 8].to_vec(); duplicate[second..second + 8].copy_from_slice(&first_id);
        let mut conflicting = template.clone();
        conflicting[field_start..field_start + 8].copy_from_slice(&99u64.to_le_bytes());
        let mut bad_ordinal = template.clone();
        let parameter = if nested { field_start + 16 + 21 + 8 } else { field_start + 16 };
        assert_eq!(bad_ordinal[parameter + 4], b'v');
        bad_ordinal[parameter + 13..parameter + 21].copy_from_slice(&99u64.to_le_bytes());
        let mut header = format!("#define NESTED {}\n", u8::from(nested));
        header.push_str(&array("selected_template_key", &selected_template));
        header.push_str(&array("selected_drop_key", &selected_drop_key));
        for (name, bytes) in [("target_key", &target), ("template_key", &template), ("later_key", &later_identity.semantic), ("later_drop_key", &later_identity.drop_template),
            ("tag_key", &tag_identity.semantic), ("tag_drop_key", &tag_identity.drop_template), ("product_key", &native.semantic), ("product_drop_key", &native.drop_template),
            ("bad_ordinal", &bad_ordinal), ("duplicate_fields", &duplicate), ("conflicting_template", &conflicting),
            ("empty_template_key", &empty_template), ("empty_product_key", &empty_identity.semantic), ("empty_drop_key", &empty_identity.drop_template)] { header.push_str(&array(name, bytes)); }
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.agent-work/fixtures").join(format!("rock-nominal-protocol-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&directory).unwrap();
        let source = super::write_sources(&directory).unwrap();
        let harness = directory.join("nominal.c");
        std::fs::write(&harness, include_str!("descriptor_nominal_protocol.c")).unwrap();
        std::fs::write(directory.join("rock_nominal_test_keys.h"), header).unwrap();
        for optimization in ["-O0", "-O3"] {
            for collisions in [false, true] {
                let executable = directory.join(format!("nominal-{optimization}-{collisions}"));
                let mut command = std::process::Command::new("cc");
                command.args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-pthread", optimization]);
                if collisions { command.arg("-DROCK_METADATA_TEST_COLLISIONS=1"); }
                let compiled = command.arg(&source).arg(&harness).arg("-o").arg(&executable).output().unwrap();
                assert!(compiled.status.success(), "nominal native compile: {}", String::from_utf8_lossy(&compiled.stderr));
                let output = std::process::Command::new(&executable).output().unwrap();
                assert!(output.status.success(), "nominal native protocol: {output:?}");
            }
        }
    }
}

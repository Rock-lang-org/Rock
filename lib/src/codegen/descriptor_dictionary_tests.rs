use std::sync::atomic::{AtomicUsize, Ordering};

use inkwell::context::Context;

use crate::ids::{CrateId, DefId, LocalDefId};
use crate::mir::*;
use crate::types::Type;

fn array(name: &str, bytes: &[u8]) -> String {
    format!("static const unsigned char {name}[] = {{{}}};\n", bytes.iter().map(|byte| byte.to_string()).collect::<Vec<_>>().join(","))
}

#[test]
fn selected_dictionary_protocol_freezes_exact_environments_and_executes_captured_read() {
    dictionary_protocol(false, false);
}

#[test]
fn polymorphic_dictionary_protocol_validates_local_binders_and_per_call_evidence() {
    dictionary_protocol(true, false);
}

#[test]
fn static_dictionary_protocol_validates_absent_receiver_and_freezes_parent_evidence() {
    dictionary_protocol(false, true);
}

fn dictionary_protocol(polymorphic: bool, static_member: bool) {
    let (mut program, producer, implementation) = if static_member {
        let program = crate::mir::static_dictionary::tests::fixture(true);
        let producer = program.backend_contract.erased.dictionary_factories.keys().next().unwrap().clone();
        let implementation = *program.backend_contract.erased.dictionary_templates.keys().next().unwrap();
        (program, producer, implementation)
    } else if polymorphic { crate::mir::polymorphic_dictionary::tests::fixture() }
        else { crate::mir::dictionary_factory::tests::fixture() };
    let context = Context::create(); let cg = super::super::CodeGen::new(&context, "dictionary_protocol");
    let mut header = array("target_key", &cg.descriptor_target_key().unwrap());
    let tc = &mut program.type_context; let contract = &mut program.backend_contract;
    let template = contract.erased.dictionary_templates[&implementation].clone();
    let template_key = dictionary_template_key(contract, tc, &template).unwrap();
    header.push_str(&array("template_key", &template_key));
    let mut bad_ordinal = template_key.clone();
    let start = bad_ordinal.windows(5).position(|w| w == b"RF01v").unwrap();
    bad_ordinal[start + 13..start + 21].copy_from_slice(&99u64.to_le_bytes());
    header.push_str(&array("bad_ordinal", &bad_ordinal));
    let mut bad_abi = template_key.clone();
    let start = bad_abi.windows(4).position(|w| w == b"DS01").unwrap(); bad_abi[start + 4] = 9;
    header.push_str(&array("bad_schema_abi", &bad_abi));
    let mut bad_retention = template_key.clone();
    let start = bad_retention.windows(4).position(|w| w == b"BQ01").unwrap();
    let size = u64::from_le_bytes(bad_retention[start - 8..start].try_into().unwrap()) as usize;
    bad_retention[start + size - 8..start + size].copy_from_slice(&1u64.to_le_bytes());
    header.push_str(&array("bad_retention", &bad_retention));
    let mut bad_returns = template_key.clone();
    bad_returns[start + 4..start + 12].copy_from_slice(&0u64.to_le_bytes());
    header.push_str(&array("bad_returns", &bad_returns));
    let mut bad_origin = template_key.clone();
    // One empty output path/source: receiver0 for Read, input1 for Forward.
    bad_origin[start + 36..start + 44].copy_from_slice(&(if polymorphic { 2u64 } else { 1 }).to_le_bytes());
    header.push_str(&array("bad_origin", &bad_origin));
    if static_member {
        fn word(bytes: &[u8], position: &mut usize) -> usize {
            let value = u64::from_le_bytes(bytes[*position..*position + 8].try_into().unwrap()) as usize;
            *position += 8; value
        }
        fn frame(bytes: &[u8], position: &mut usize) { let size = word(bytes, position); *position += size; }
        let mut position = template_key.windows(4).position(|w| w == b"DS01").unwrap() + 12;
        frame(&template_key, &mut position); frame(&template_key, &mut position);
        let arguments = word(&template_key, &mut position);
        for _ in 0..arguments { frame(&template_key, &mut position); }
        assert_eq!(word(&template_key, &mut position), 1);
        frame(&template_key, &mut position);
        for (label, receiver) in [("bad_shared", 0u64), ("bad_move", 2), ("bad_receiver_kind", 4)] {
            let mut bad = template_key.clone(); bad[position..position + 8].copy_from_slice(&receiver.to_le_bytes());
            header.push_str(&array(label, &bad));
        }
        assert_eq!(word(&template_key, &mut position), 3);
        let mut bad = template_key.clone(); bad[position..position + 8].copy_from_slice(&1u64.to_le_bytes());
        header.push_str(&array("bad_static_local", &bad));
        assert_eq!(word(&template_key, &mut position), 0); assert_eq!(word(&template_key, &mut position), 0);
        let mut bad = template_key.clone(); bad[position..position + 8].copy_from_slice(&2u64.to_le_bytes());
        header.push_str(&array("bad_static_arity", &bad));
        assert_eq!(word(&template_key, &mut position), 1);
        let argument_start = position; frame(&template_key, &mut position);
        let argument = template_key[argument_start..position].to_vec();
        let result_start = position; frame(&template_key, &mut position);
        let mut bad = template_key.clone(); bad.splice(result_start..position, argument);
        let schema = template_key.windows(4).position(|w| w == b"DS01").unwrap();
        let old_size = u64::from_le_bytes(template_key[schema - 8..schema].try_into().unwrap()) as usize;
        let new_size = old_size + bad.len() - template_key.len();
        bad[schema - 8..schema].copy_from_slice(&(new_size as u64).to_le_bytes());
        header.push_str(&array("bad_static_result", &bad));
    }
    if polymorphic {
        let mut bad_local = template_key.clone();
        let start = bad_local.windows(5).position(|w| w == b"PM01m").unwrap();
        bad_local[start + 13..start + 21].copy_from_slice(&1u64.to_le_bytes());
        header.push_str(&array("bad_local", &bad_local));
        let start = template_key.windows(5).position(|w| w == b"PM01c").unwrap();
        let mut bad_kind = template_key.clone(); bad_kind[start - 32..start - 24].copy_from_slice(&1u64.to_le_bytes());
        header.push_str(&array("bad_kind", &bad_kind));
        let mut bad_bounds = template_key.clone(); bad_bounds[start - 24..start - 16].copy_from_slice(&1u64.to_le_bytes());
        header.push_str(&array("bad_bounds", &bad_bounds));
    }
    // The static production failure used an RK01 nominal Self, not the scalar
    // Self exercised by the original protocol harness. Use the real producer
    // vtable's single-I64-field Service identity and complete drop children.
    let second_type = if static_member {
        let table = contract.vtables.values().next().unwrap();
        tc.type_for(table.concrete)
    } else { Type::U64 };
    for (label, ty) in [("a", Type::I64), ("b", second_type)] {
        let concrete = tc.intern_type(&ty);
        let drop = payload_drop_plan(contract, tc, concrete).unwrap();
        if static_member && label == "b" {
            assert!(matches!(ty, Type::Struct { .. }));
            let MirPayloadDropChildren::Fields(fields) = &drop.children else { panic!("actual nominal drop fields missing") };
            assert_eq!(fields.len(), 1);
            assert_eq!(tc.type_for(fields[0].ty), Type::I64);
        }
        let identity = descriptor_identity::descriptor_identity(contract, tc, &drop).unwrap();
        header.push_str(&array(&format!("type_{label}_key"), &identity.semantic));
        header.push_str(&array(&format!("type_{label}_drop"), &identity.drop_template));
        let argument = MirErasedType::Concrete(concrete);
        let read = if static_member { &template.bounds[0] } else { &contract.erased.functions[&producer].signature.dictionaries[0] }
            .substitute(&[argument.clone()], tc).unwrap();
        let read_key = dictionary_schema_key(contract, tc, &read, true).unwrap();
        let method = DefId::new(CrateId(0), LocalDefId(51));
        let member_id = read.members[0].member_id;
        if static_member {
            contract.dictionary_impl_members.insert(DefId::new(CrateId(0), LocalDefId(50)), MirDictionaryImplAuthority {
                trait_id: read.trait_id, members: [(member_id, method)].into_iter().collect(), factory_compatible: true, template: None,
            });
        }
        let native_template = MirDictionaryFactoryTemplate { implementation: DefId::new(CrateId(0), LocalDefId(50)), parameters: vec![], result: read, bounds: vec![],
            members: vec![MirDictionaryFactoryMember { member_id, method_id: method, entry: MirErasedKey { definition: method, static_args: vec![] } }] };
        let mut native_binding = b"DI01".to_vec();
        dictionary_frame(&mut native_binding, &read_key); dictionary_frame(&mut native_binding, &dictionary_template_key(contract, tc, &native_template).unwrap());
        native_binding.extend_from_slice(&[0; 16]);
        header.push_str(&array(&format!("read_{label}_key"), &read_key)); header.push_str(&array(&format!("read_{label}_binding"), &native_binding));
        let relay_key = dictionary_schema_key(contract, tc, &template.result.substitute(&[argument], tc).unwrap(), true).unwrap();
        let mut binding = b"DI01".to_vec(); dictionary_frame(&mut binding, &relay_key); dictionary_frame(&mut binding, &template_key);
        binding.extend_from_slice(&1u64.to_le_bytes()); dictionary_frame(&mut binding, &identity.semantic); dictionary_frame(&mut binding, &identity.drop_template);
        binding.extend_from_slice(&1u64.to_le_bytes()); dictionary_frame(&mut binding, &read_key); dictionary_frame(&mut binding, &native_binding);
        header.push_str(&array(&format!("relay_{label}_key"), &relay_key)); header.push_str(&array(&format!("relay_{label}_binding"), &binding));
    }
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.agent-work/fixtures").join(format!("rock-dictionary-protocol-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&directory).unwrap();
    let source = super::write_sources(&directory).unwrap(); let harness = directory.join("dictionary.c");
    std::fs::write(&harness, if static_member { include_str!("descriptor_static_dictionary_protocol.c") }
        else if polymorphic { include_str!("descriptor_polymorphic_dictionary_protocol.c") }
        else { include_str!("descriptor_dictionary_protocol.c") }).unwrap();
    std::fs::write(directory.join("rock_dictionary_test_keys.h"), header).unwrap();
    for optimization in ["-O0", "-O3"] {
        for collisions in [false, true] {
            let executable = directory.join(format!("dictionary-{optimization}-{collisions}"));
            let mut command = std::process::Command::new("cc"); command.args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-pthread", optimization]);
            if collisions { command.arg("-DROCK_METADATA_TEST_COLLISIONS=1"); }
            let compiled = command.arg(&source).arg(&harness).arg("-o").arg(&executable).output().unwrap();
            assert!(compiled.status.success(), "dictionary C compile: {}", String::from_utf8_lossy(&compiled.stderr));
            let output = std::process::Command::new(&executable).output().unwrap(); assert!(output.status.success(), "dictionary C protocol: {output:?}");
        }
    }
}

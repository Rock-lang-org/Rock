use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use inkwell::context::Context;

use crate::ids::{CrateId, DefId, LocalDefId};
use crate::mir::descriptor_identity::{descriptor_identity, tuple_drop_template, tuple_identity};
use crate::mir::*;
use crate::products::ProductCrateIdentity;
use crate::type_context::TypeContext;
use crate::types::Type;

fn array(name: &str, bytes: &[u8]) -> String {
    format!("static const unsigned char {name}[] = {{{}}};\n", bytes.iter().map(|byte| byte.to_string()).collect::<Vec<_>>().join(","))
}

#[test]
fn native_runtime_protocol_executes_exact_codec_drop_and_concurrency_controls() {
    let context = Context::create();
    let cg = super::super::CodeGen::new(&context, "descriptor_protocol");
    let target = cg.descriptor_target_key().unwrap();
    let mut contract = MirBackendContract::default();
    contract.descriptor_origins.insert(CrateId(1), ProductCrateIdentity::local("metadata-protocol".into()));
    let mut tc = TypeContext::new();
    let unit = tc.intern_type(&Type::Unit);
    let mut keys = BTreeMap::new();
    let mut header = array("target_key", &target);
    for (ordinal, name) in [(1, "later"), (2, "other"), (3, "over"), (4, "huge")] {
        let id = DefId::new(CrateId(1), LocalDefId(ordinal));
        let ty = tc.intern_type(&Type::Struct { id, args: Vec::new() });
        let key = MirCallableKey::Extern(DefId::new(CrateId(1), LocalDefId(ordinal + 10)));
        contract.callables.insert(key.clone(), MirCallableDecl {
            key: key.clone(), source_def_id: Some(DefId::new(CrateId(1), LocalDefId(ordinal + 10))),
            kind: MirCallableKind::Extern { link_name: format!("{name}_drop"), variadic: false },
            llvm_symbol: format!("{name}_drop"), linkage: MirLinkage::External,
            signature: MirCallableSignature::from_type_ids(&[ty], unit, MirPassMode::Pointer),
        });
        let identity = descriptor_identity(&contract, &tc, &MirPayloadDropPlan { ty, user_drop: Some(key), children: MirPayloadDropChildren::None }).unwrap();
        header.push_str(&array(&format!("{name}_key"), &identity.semantic));
        header.push_str(&array(&format!("{name}_drop"), &identity.drop_template));
        keys.insert(name, identity);
        if name == "later" {
            let alternate = descriptor_identity(&contract, &tc, &MirPayloadDropPlan { ty, user_drop: None, children: MirPayloadDropChildren::None }).unwrap();
            header.push_str(&array("later_alt_drop", &alternate.drop_template));
        }
    }
    let zero = descriptor_identity(&contract, &tc, &MirPayloadDropPlan { ty: unit, user_drop: None, children: MirPayloadDropChildren::None }).unwrap();
    header.push_str(&array("zero_key", &zero.semantic));
    header.push_str(&array("zero_drop", &zero.drop_template));
    let pair_key = tuple_identity(&[keys["later"].semantic.clone(), keys["later"].semantic.clone()]);
    let pair_drop = tuple_drop_template(&[keys["later"].drop_template.clone(), keys["later"].drop_template.clone()]);
    header.push_str(&array("pair_key", &pair_key));
    header.push_str(&array("pair_drop", &pair_drop));
    header.push_str(&array("nested_key", &tuple_identity(&[pair_key, keys["other"].semantic.clone()])));
    header.push_str(&array("nested_drop", &tuple_drop_template(&[pair_drop, keys["other"].drop_template.clone()])));
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.agent-work/fixtures").join(format!("rock-metadata-protocol-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&directory).unwrap();
    let source = super::write_sources(&directory).unwrap();
    let harness = directory.join("protocol.c");
    std::fs::write(&harness, include_str!("descriptor_runtime_protocol.c")).unwrap();
    std::fs::write(directory.join("rock_descriptor_test_keys.h"), header).unwrap();
    // Serialized real C compilation/execution; no compiler-host substitute.
    for optimization in ["-O0", "-O3"] {
        for collisions in [false, true] {
            let executable = directory.join(format!("protocol-{optimization}-{collisions}"));
            let mut command = std::process::Command::new("cc");
            command.args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-pthread", optimization]);
            if collisions { command.arg("-DROCK_METADATA_TEST_COLLISIONS=1"); }
            let compiled = command.arg(&source).arg(&harness).arg("-o").arg(&executable).output().unwrap();
            assert!(compiled.status.success(), "native protocol compile: {}", String::from_utf8_lossy(&compiled.stderr));
            let executed = std::process::Command::new(&executable).output().unwrap();
            assert!(executed.status.success(), "native protocol failed: {:?}: {}", executed.status, String::from_utf8_lossy(&executed.stderr));
        }
    }
    // Keep the small generated sources/binaries for coordinator diagnostics.
}

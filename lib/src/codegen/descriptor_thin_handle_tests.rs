use std::sync::atomic::{AtomicUsize, Ordering};

use inkwell::context::Context;

use crate::ids::{CrateId, DefId, FieldId, LocalDefId};
use crate::mir::*;
use crate::types::{GenericParamId, Type};

fn array(name: &str, bytes: &[u8]) -> String {
    format!("static const unsigned char {name}[] = {{{}}};\n",
        bytes.iter().map(|byte| byte.to_string()).collect::<Vec<_>>().join(","))
}

#[test]
fn native_thin_handle_templates_check_access_pointee_identity_and_frozen_drop_at_o0_o3() {
    for access in 0..3 {
        let context = Context::create();
        let codegen = crate::codegen::CodeGen::new(&context, "thin_handle_protocol");
        let target = codegen.descriptor_target_key().unwrap();
        let mut tc = crate::type_context::TypeContext::new();
        let mut contract = MirBackendContract::default();
        contract.descriptor_origins.insert(CrateId(0), crate::products::ProductCrateIdentity::local("thin_handle_protocol".into()));
        let owner = DefId::new(CrateId(0), LocalDefId(100 + access));
        let parameter = GenericParamId { owner, index: 0 };
        let handle = |ty| match access {
            0 => Type::Pointer(Box::new(ty)),
            _ => Type::Reference { inner: Box::new(ty), mutable: access == 2 },
        };
        let field = tc.intern_type(&handle(Type::Generic(parameter)));
        contract.nominal_layouts.insert(owner, MirNominalLayout::Struct {
            id: owner, fields: vec![("address".into(), field)], generic_params: vec![parameter],
        });
        contract.nominal_field_ids.insert(owner, vec![FieldId(0)]);
        contract.nominal_drop_authority.insert(owner, MirNominalDropAuthority::FieldsOnly);
        let scalar = tc.intern_type(&Type::I64);
        let scalar_drop = payload_drop_plan(&contract, &mut tc, scalar).unwrap();
        let scalar_identity = descriptor_identity::descriptor_identity(&contract, &tc, &scalar_drop).unwrap();
        let handle_ty = tc.intern_type(&handle(Type::I64));
        let handle_drop = payload_drop_plan(&contract, &mut tc, handle_ty).unwrap();
        let handle_identity = descriptor_identity::descriptor_identity(&contract, &tc, &handle_drop).unwrap();
        let wrong_ty = tc.intern_type(&handle(Type::U64));
        let wrong_drop = payload_drop_plan(&contract, &mut tc, wrong_ty).unwrap();
        let wrong_identity = descriptor_identity::descriptor_identity(&contract, &tc, &wrong_drop).unwrap();
        let wrong_access_ty = tc.intern_type(&Type::Reference { inner: Box::new(Type::I64), mutable: access != 2 });
        let wrong_access_drop = payload_drop_plan(&contract, &mut tc, wrong_access_ty).unwrap();
        let wrong_access = descriptor_identity::descriptor_identity(&contract, &tc, &wrong_access_drop).unwrap();
        let product_ty = tc.intern_type(&Type::Struct { id: owner, args: vec![Type::I64] });
        let product_drop = MirPayloadDropPlan {
            ty: product_ty, user_drop: None, children: MirPayloadDropChildren::Fields(vec![handle_drop]),
        };
        let product = descriptor_identity::descriptor_identity(&contract, &tc, &product_drop).unwrap();
        let (template, _) = nominal_template(&contract, &tc, owner).unwrap();
        let mut header = String::new();
        for (name, bytes) in [
            ("target_key", &target), ("template_key", &template),
            ("scalar_key", &scalar_identity.semantic), ("scalar_drop_key", &scalar_identity.drop_template),
            ("handle_key", &handle_identity.semantic), ("handle_drop_key", &handle_identity.drop_template),
            ("wrong_key", &wrong_identity.semantic), ("wrong_drop_key", &wrong_identity.drop_template),
            ("wrong_access_key", &wrong_access.semantic), ("wrong_access_drop_key", &wrong_access.drop_template),
            ("product_key", &product.semantic), ("product_drop_key", &product.drop_template),
        ] { header.push_str(&array(name, bytes)); }
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.agent-work/fixtures")
            .join(format!("thin-handle-protocol-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&root).unwrap();
        let runtime = super::write_sources(&root).unwrap();
        let source = root.join("thin_handle.c");
        std::fs::write(&source, format!(r#"
#include "rock_descriptor_runtime.h"
#include <assert.h>
#include <string.h>
{header}
static unsigned payload_drops, handle_drops;
static void payload_drop(void *value, const RockDescriptor *descriptor) {{
    (void)value; (void)descriptor; ++payload_drops;
}}
static void handle_drop(void *value, const RockDescriptor *descriptor) {{
    (void)value; (void)descriptor; ++handle_drops;
}}
int main(void) {{
    RockDescriptorTarget target = {{ROCK_DESCRIPTOR_ABI, 64, 1, INT64_MAX, sizeof(target_key), target_key}};
    RockNominalTemplate nominal = {{ROCK_DESCRIPTOR_ABI, sizeof(template_key), template_key, 0, NULL, NULL}};
    RockDescriptor scalar = {{
        .size = 8, .alignment = 8, .drop = payload_drop, .abi = ROCK_DESCRIPTOR_ABI, .kind = 1, .state = 2,
        .key_size = sizeof(scalar_key), .key = scalar_key,
        .drop_key_size = sizeof(scalar_drop_key), .drop_key = scalar_drop_key,
        .target_key_size = sizeof(target_key), .target_key = target_key, .index_limit = INT64_MAX,
    }};
    RockDescriptor handle = scalar;
    handle.drop = handle_drop;
    handle.key_size = sizeof(handle_key); handle.key = handle_key;
    handle.drop_key_size = sizeof(handle_drop_key); handle.drop_key = handle_drop_key;
    const RockDescriptor *arguments[] = {{&scalar}}, *fields[] = {{&handle}};
    const RockDescriptor *result = NULL, *again = NULL;
    assert(rock_descriptor_nominal(&target, &nominal, arguments, fields, &result) == ROCK_METADATA_OK);
    assert(result && result->size == 8 && result->alignment == 8 && result->field_count == 1);
    assert(result->key_size == sizeof(product_key) && !memcmp(result->key, product_key, sizeof(product_key)));
    assert(result->drop_key_size == sizeof(product_drop_key) && !memcmp(result->drop_key, product_drop_key, sizeof(product_drop_key)));
    assert(rock_descriptor_nominal(&target, &nominal, arguments, fields, &again) == ROCK_METADATA_OK && again == result);
    unsigned char storage[8] = {{0}};
    result->drop(storage, result);
    assert(handle_drops == 1 && payload_drops == 0);
    RockDescriptor wrong = handle;
    wrong.key_size = sizeof(wrong_key); wrong.key = wrong_key;
    wrong.drop_key_size = sizeof(wrong_drop_key); wrong.drop_key = wrong_drop_key;
    fields[0] = &wrong; again = result;
    assert(rock_descriptor_nominal(&target, &nominal, arguments, fields, &again) == ROCK_METADATA_INVALID && !again);
    wrong.key_size = sizeof(wrong_access_key); wrong.key = wrong_access_key;
    wrong.drop_key_size = sizeof(wrong_access_drop_key); wrong.drop_key = wrong_access_drop_key;
    again = result;
    assert(rock_descriptor_nominal(&target, &nominal, arguments, fields, &again) == ROCK_METADATA_INVALID && !again);
    /* The interner owns its evidence, not this mutable request storage. */
    memset(&handle, 0, sizeof(handle));
    assert(result->children[0]->key_size == sizeof(handle_key));
    assert(!memcmp(result->children[0]->key, handle_key, sizeof(handle_key)));
    result->drop(storage, result);
    assert(handle_drops == 2 && payload_drops == 0);
    return 0;
}}
"#)).unwrap();
        for optimization in ["-O0", "-O3"] {
            for collisions in [false, true] {
                let executable = root.join(format!("thin-handle-{optimization}-{collisions}"));
                let mut command = std::process::Command::new("cc");
                command.args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-pthread", optimization]);
                if collisions { command.arg("-DROCK_METADATA_TEST_COLLISIONS=1"); }
                let output = command.arg(&runtime).arg(&source).arg("-o").arg(&executable).output().unwrap();
                assert!(output.status.success(), "native handle compile: {output:?}");
                let output = std::process::Command::new(executable).output().unwrap();
                assert!(output.status.success(), "access={access}: native handle protocol: {output:?}");
            }
        }
    }
}

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::hir::borrowing::NativeReturnSource;
use crate::{Config, SourceProvider};

const PROVIDER: &str = r#"
< struct Cursor
    < text: &Str
< struct NumberCursor
    < value: &I64
new: &Str -> Cursor
< new = text -> Cursor
    text: text
copied: & &Str -> Cursor
< copied = metadata -> new *metadata
slot: & &Str -> & &Str
< slot = metadata -> metadata
copy_number: & &I64 -> NumberCursor
< copy_number = metadata -> NumberCursor
    value: *metadata
"#;

fn provider() -> (PathBuf, CompilerProducts, PathBuf) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".agent-work/fixtures")
        .join(format!(
            "native-borrow-wire-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("absent-provider.rk");
    let config = Config {
        entry_file: source.clone(),
        output_dir: root.clone(),
        current_crate_name: Some("views".into()),
        no_std: true,
        no_prelude: true,
        no_link: true,
        emit_object: Some(root.join("views.o")),
        source_providers: vec![SourceProvider::Virtual {
            path: source.clone(),
            text: PROVIDER.into(),
        }],
        ..Default::default()
    };
    let mut products = crate::compile_with_products(&config)
        .unwrap()
        .products
        .unwrap();
    products.link.object_path = Some("views.o".into());
    assert!(!source.exists());
    assert!(!products.link.native_return_origins.is_empty());
    for (id, origins) in &products.link.native_return_origins {
        assert!(
            origins.sources_are_valid(|ty| matches!(ty, Type::Reference { .. } | Type::Pointer(_)))
        );
        assert!(!products.bodies.functions.contains_key(id));
    }
    let artifact = root.join("views.rkca");
    products.write_artifact_to_path(&artifact).unwrap();
    (root, products, artifact)
}

#[test]
fn native_return_contract_source_free_execution_and_storage_escape_negative() {
    let (root, products, artifact) = provider();
    let decoded = CompilerProducts::read_artifact_from_path(&artifact).unwrap();
    assert_eq!(
        decoded.link.native_return_origins,
        products.link.native_return_origins
    );
    let frozen = std::fs::read(&artifact).unwrap();
    let object = std::fs::read(root.join("views.o")).unwrap();
    for opt_level in [0, 3] {
        let entry = root.join(format!("caller{opt_level}.rk"));
        let output_dir = root.join(format!("caller{opt_level}"));
        std::fs::create_dir(&output_dir).unwrap();
        let source = r#"
> views::copied
main = ->
    cursor = copied &"é🦀"
    ~ArrayLen cursor.text
"#;
        let config = Config {
            entry_file: entry.clone(),
            output_dir: output_dir.clone(),
            opt_level,
            extern_artifacts: vec![("views".into(), artifact.clone())],
            source_providers: vec![SourceProvider::Virtual {
                path: entry,
                text: source.into(),
            }],
            no_std: true,
            no_prelude: true,
            ..Default::default()
        };
        crate::compile(&config).unwrap();
        let output = std::process::Command::new(output_dir.join(format!("caller{opt_level}")))
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(6));
        assert!(output.stdout.is_empty() && output.stderr.is_empty());
    }
    let entry = root.join("escape.rk");
    let source = r#"
> views::slot
escape: () -> & &Str
escape = ->
    local = "é🦀"
    slot &local
main = !->
    held = escape!
"#;
    let diagnostics = crate::compile(&Config {
        entry_file: entry.clone(),
        output_dir: root.clone(),
        no_link: true,
        no_std: true,
        no_prelude: true,
        extern_artifacts: vec![("views".into(), artifact.clone())],
        source_providers: vec![SourceProvider::Virtual {
            path: entry.clone(),
            text: source.into(),
        }],
        ..Default::default()
    })
    .unwrap_err();
    assert!(!diagnostics.0.is_empty());
    assert!(diagnostics.0.iter().all(|diagnostic| diagnostic.code
        == Some(crate::diagnostic::DiagnosticCode::Borrow)
        && diagnostic
            .primary
            .as_ref()
            .is_some_and(|primary| primary.span.file_path == entry)));
    assert_eq!(std::fs::read(&artifact).unwrap(), frozen);
    assert_eq!(std::fs::read(root.join("views.o")).unwrap(), object);

    let entry = root.join("payload-escape.rk");
    let source = r#"
> views::NumberCursor
> views::copy_number
escape: () -> NumberCursor
escape = ->
    local = 7
    copy_number (& (&local))
main = !->
    held = escape!
"#;
    let diagnostics = crate::compile(&Config {
        entry_file: entry.clone(),
        output_dir: root.clone(),
        no_link: true,
        no_std: true,
        no_prelude: true,
        extern_artifacts: vec![("views".into(), artifact.clone())],
        source_providers: vec![SourceProvider::Virtual {
            path: entry.clone(),
            text: source.into(),
        }],
        ..Default::default()
    })
    .unwrap_err();
    assert!(!diagnostics.0.is_empty());
    assert!(diagnostics.0.iter().all(|diagnostic| diagnostic.code
        == Some(crate::diagnostic::DiagnosticCode::Borrow)
        && diagnostic
            .primary
            .as_ref()
            .is_some_and(|primary| primary.span.file_path == entry)));
    assert_eq!(std::fs::read(&artifact).unwrap(), frozen);
    assert_eq!(std::fs::read(root.join("views.o")).unwrap(), object);

    let mut opaque = products;
    opaque.link.native_return_origins.clear();
    let opaque_artifact = root.join("opaque.rkca");
    opaque.write_artifact_to_path(&opaque_artifact).unwrap();
    let entry = root.join("missing-authority.rk");
    let diagnostics = crate::compile(&Config {
        entry_file: entry.clone(),
        output_dir: root.clone(),
        no_link: true,
        no_std: true,
        no_prelude: true,
        extern_artifacts: vec![("views".into(), opaque_artifact)],
        source_providers: vec![SourceProvider::Virtual {
            path: entry.clone(),
            text: "> views::copied\nmain = !->\n    cursor = copied &\"é🦀\"\n".into(),
        }],
        ..Default::default()
    })
    .unwrap_err();
    assert!(!diagnostics.0.is_empty());
    assert!(diagnostics.0.iter().all(|diagnostic| diagnostic.code
        == Some(crate::diagnostic::DiagnosticCode::Borrow)
        && diagnostic
            .primary
            .as_ref()
            .is_some_and(|primary| primary.span.file_path == entry)));
    assert_eq!(std::fs::read(&artifact).unwrap(), frozen);
    assert_eq!(std::fs::read(root.join("views.o")).unwrap(), object);
}

#[test]
fn native_return_contract_artifact_rejects_malformed_authority_at_decode() {
    let (_, original, _) = provider();
    let id = *original
        .link
        .native_return_origins
        .iter()
        .find(|(_, origins)| origins.sources == [NativeReturnSource::ArgumentContents(0)])
        .unwrap()
        .0;
    for mutation in 0..9 {
        let mut products = original.clone();
        match mutation {
            0 => {
                products.link.records.remove(&id);
            }
            1 => products
                .link
                .native_return_origins
                .get_mut(&id)
                .unwrap()
                .sources
                .clear(),
            2 => {
                products
                    .link
                    .native_return_origins
                    .get_mut(&id)
                    .unwrap()
                    .sources = vec![NativeReturnSource::ArgumentContents(1)]
            }
            3 => {
                products
                    .link
                    .native_return_origins
                    .get_mut(&id)
                    .unwrap()
                    .sources = vec![NativeReturnSource::ArgumentContents(0); 2]
            }
            4 => {
                products
                    .link
                    .native_return_origins
                    .get_mut(&id)
                    .unwrap()
                    .params[0] = Type::I64
            }
            5 => {
                products
                    .link
                    .native_return_origins
                    .get_mut(&id)
                    .unwrap()
                    .ret = Type::I64
            }
            6 => products.link.object_path = None,
            7 => products
                .link
                .records
                .get_mut(&id)
                .unwrap()
                .backend_symbol
                .clear(),
            8 => {
                products
                    .link
                    .native_return_origins
                    .get_mut(&id)
                    .unwrap()
                    .sources = vec![NativeReturnSource::UnknownExternal]
            }
            _ => unreachable!(),
        }
        assert!(
            products.to_artifact_bytes().is_err(),
            "emission mutation {mutation}"
        );
        // Bypass emission validation to exercise the untrusted decode boundary.
        let artifact = super::super::type_table::products_to_artifact(
            &products,
            PRODUCT_ARTIFACT_FORMAT_VERSION,
        )
        .unwrap();
        let header = ProductArtifactHeader::from_products(&products);
        let bytes = encode_product_artifact(&header, &artifact).unwrap();
        assert!(
            CompilerProducts::from_artifact_bytes(&bytes).is_err(),
            "decode mutation {mutation}"
        );
    }
}

#[test]
fn early_return_native_storage_schema_corruption_rejects_at_emission_and_decode() {
    let (root, _, _) = provider();
    let source = root.join("absent-early-provider.rk");
    let text = r#"
trait CopyValue
    ~@into_value: I64
impl CopyValue for I64
    ~@into_value = -> self
< struct Holder
    < value: () -> I64
< enum Choice
    Empty
    Held Holder
make: Bool -> I64 -> Holder
< make = flag, seed ->
    if flag
        return (Holder
            value: -> seed.into_value!)
    Holder
        value: -> seed.into_value!
make_choice: Bool -> I64 -> Choice
< make_choice = flag, seed ->
    if flag
        return (Choice::Held (make true, seed))
    Choice::Held (make false, seed)
"#;
    let config = Config {
        entry_file: source.clone(), output_dir: root.clone(),
        current_crate_name: Some("early".into()), no_std: true,
        no_prelude: true, no_link: true, emit_object: Some(root.join("early.o")),
        source_providers: vec![SourceProvider::Virtual { path: source.clone(), text: text.into() }],
        ..Default::default()
    };
    let mut original = crate::compile_with_products(&config).unwrap().products.unwrap();
    original.link.object_path = Some("early.o".into());
    let find = |name: &str| *original.interface.functions.iter()
        .find(|(_, function)| function.name.rsplit("::").next() == Some(name)).unwrap().0;
    let holder = find("make");
    let choice = find("make_choice");
    assert!(!original.bodies.functions.contains_key(&holder) && !original.bodies.functions.contains_key(&choice));
    let frozen = original.to_artifact_bytes().unwrap();
    let object = std::fs::read(root.join("early.o")).unwrap();
    CompilerProducts::from_artifact_bytes(&frozen).unwrap();
    assert!(!source.exists());
    for mutation in 0..7 {
        let mut products = original.clone();
        if mutation < 4 {
            let Type::StructRepresentation { id, args, fields } =
                &mut products.interface.functions.get_mut(&holder).unwrap().ret_type else { panic!("measured struct producer") };
            match mutation {
                0 => fields.clear(),
                1 => fields.push(Type::I64),
                2 => {
                    let Type::Function { ret, .. } = &mut fields[0] else { panic!("measured callable field") };
                    **ret = Type::Bool;
                }
                3 => {
                    *id = crate::ids::DefId::new(crate::ids::CrateId(0), crate::ids::LocalDefId(u32::MAX));
                    args.push(Type::I64);
                }
                _ => unreachable!(),
            }
        } else {
            let Type::EnumRepresentation { variants, .. } =
                &mut products.interface.functions.get_mut(&choice).unwrap().ret_type else { panic!("measured enum producer") };
            match mutation {
                4 => variants.push(vec![]),
                5 => { variants.remove(0); }
                6 => variants.iter_mut().find(|fields| !fields.is_empty()).unwrap().clear(),
                _ => unreachable!(),
            };
        }
        assert!(products.to_artifact_bytes().is_err(), "emission mutation {mutation}");
        let artifact = super::super::type_table::products_to_artifact(&products, PRODUCT_ARTIFACT_FORMAT_VERSION).unwrap();
        let header = ProductArtifactHeader::from_products(&products);
        let bytes = encode_product_artifact(&header, &artifact).unwrap();
        assert!(CompilerProducts::from_artifact_bytes(&bytes).is_err(), "decode mutation {mutation}");
        assert_eq!(original.to_artifact_bytes().unwrap(), frozen);
        assert_eq!(std::fs::read(root.join("early.o")).unwrap(), object);
        assert!(!source.exists());
    }
}

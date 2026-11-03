use super::*;

fn compile_artifact_app_without_std_failure(
    source: &str,
    extern_artifacts: Vec<(String, PathBuf)>,
) -> crate::diagnostic::Diagnostics {
    let temp_dir = temp_test_dir("artifact_no_std_failure");
    let _cleanup = TestDirCleanup(temp_dir.clone());
    let entry_file = temp_dir.join("main.rk");
    fs::write(&entry_file, source).unwrap();

    crate::compile(&Config {
        entry_file,
        output_dir: temp_dir.join("out"),
        debug_print: vec![],
        meta_files: vec![],
        extern_artifacts,
        source_providers: Vec::new(),
        current_crate_name: None,
        opt_level: 0,
        overflow_checks: None,
        emit_llvm: false,
        no_link: true,
        emit_object: None,
        no_prelude: true,
        no_std: true,
        sysroot: None,
    })
    .expect_err("no-std compilation should fail")
}

#[test]
fn renamed_language_item_provider_artifact_drives_all_protocols() {
    let temp_dir = temp_test_dir("renamed_language_item_provider");
    let provider_dir = temp_dir.join("protocols");
    let provider_artifact = temp_dir.join("protocols.rkca");

    write_crate(
        &provider_dir,
        "protocols",
        "",
        r#"lang sized
< trait Stature
< Stature

lang drop
< trait Farewell
    lang method
    ~@dismiss: ()
< Farewell

lang index
< trait Slot Key
    lang output
    type Yield
    lang method
    @fetch: Key -> &Self::Yield
< Slot

lang control_flow
< enum Fork B, C
    lang break
    Stop B
    lang continue
    Go C
< Fork

lang try
< trait Passage
    lang output
    type Resulting
    lang residual
    type Debris
    lang branch
    ~@divide: Fork Self::Debris, Self::Resulting
< Passage

lang from_residual
< trait Rebuild R
    lang method
    heal: R -> Self
< Rebuild

extern puts: *U8 -> I32

report_drop: &[U8] -> ()
report_drop = bytes ->
    puts (~ArrPtr *bytes)
    return
< report_drop

struct Ticket

impl Farewell for Ticket
    ~@dismiss = ->
        report_drop (&[100, 114, 111, 112, 0])
        return

make_ticket = -> Ticket
< Ticket
< make_ticket

struct Rack
    < value: I64

impl Slot I64 for Rack
    type Yield = I64
    @fetch = _ -> &@value

make_rack = ->
    Rack
        value: 1
< Rack
< make_rack

enum Outcome T
    Ready T
    Stalled
< Outcome

enum Residue
    Stalled I64

impl Passage for Outcome T
    type Resulting = T
    type Debris = Residue

    ~@divide = ->
        match self
            Outcome::Ready value => Fork::Go value
            Outcome::Stalled => Fork::Stop (Residue::Stalled 7)

impl Rebuild Residue for Outcome T
    heal = residue ->
        match residue
            Residue::Stalled _ => Outcome::Stalled

candidate: Bool -> Outcome I64
candidate = ok ->
    if ok
        Outcome::Ready 41
    else
        Outcome::Stalled
< candidate
"#,
    );
    build_product_artifact(&provider_dir, "protocols", &provider_artifact, true, true);

    // An explicit consuming Drop method call must link and consume the ticket exactly once.
    let (explicit_stdout, explicit_exit_code) = compile_and_run_artifact_app(
        r#"> protocols::*

extern exit: I32 -> ()

requires_stature: T -> I64 where T: Stature
requires_stature = _ -> 1

compute: Bool -> Outcome I64
compute = ok ->
    value = candidate ok?
    Outcome::Ready 42

main: () -> I64
main = ->
    ticket = make_ticket!
    rack = Rack
        value: 42
    indexed = rack[0]
    ticket.dismiss!
    layout = requires_stature rack
    success = compute true
    failure = compute false
    exit 42
    0
        "#,
        vec![("protocols".to_string(), provider_artifact.clone())],
    );

    assert_eq!(explicit_stdout, "drop\n");
    assert_eq!(explicit_exit_code, 42);

    // Normal scope return must invoke the marked Drop method without an explicit call.
    let (automatic_stdout, automatic_exit_code) = compile_and_run_artifact_app(
        r#"> protocols::*

main: () -> I64
main = ->
    ticket = make_ticket!
    42
"#,
        vec![("protocols".to_string(), provider_artifact)],
    );

    assert_eq!(automatic_stdout, "drop\n");
    assert_eq!(automatic_exit_code, 42);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn renamed_artifact_index_and_index_mut_drive_downstream_syntax() {
    let temp_dir = temp_test_dir("renamed_artifact_index_pair");
    let provider_dir = temp_dir.join("provider");
    let provider_artifact = temp_dir.join("provider.rkca");

    write_crate(
        &provider_dir,
        "provider",
        "",
        r#"lang index
< trait ReadAt Key
    lang output
    type ReadValue
    lang method
    @read_at: Key -> &Self::ReadValue
< ReadAt

lang index_mut
< trait WriteAt Key
    lang output
    type WriteValue
    lang method
    ^@write_at: Key -> &mut Self::WriteValue
< WriteAt
"#,
    );
    build_product_artifact(&provider_dir, "provider", &provider_artifact, true, true);

    let downstream_source = r#"> provider::*

struct Cell
    < read_value: I64
    < write_value: I64

impl ReadAt I64 for Cell
    type ReadValue = I64
    @read_at = _ -> &@read_value

impl WriteAt I64 for Cell
    type WriteValue = I64
    ^@write_at = _ -> &mut @write_value

probe: T -> I64
probe = _ ->
    mut target = Cell
        read_value: 1
        write_value: 0
    source = Cell
        read_value: 7
        write_value: 2
    target[0] = source[0]
    target.write_value

main: () -> I64
main = -> probe 0
"#;

    let (stdout, exit_code) = compile_and_run_artifact_app(
        downstream_source,
        vec![("provider".to_string(), provider_artifact.clone())],
    );
    assert_eq!(stdout, "");
    assert_eq!(exit_code, 7);

    let provider_products = CompilerProducts::read_artifact_from_path(&provider_artifact)
        .expect("provider artifact should load for ID assertions");
    let provider_read = provider_products
        .interface
        .language_items
        .index
        .as_ref()
        .expect("provider should mark ReadAt as Index");
    let provider_write = provider_products
        .interface
        .language_items
        .index_mut
        .as_ref()
        .expect("provider should mark WriteAt as IndexMut");

    let mut context = CrateContext::new();
    context
        .load_product_artifact_from_path_as("provider", provider_artifact.clone())
        .expect("provider artifact should load into a fresh context");
    let runtime_provider = context
        .extern_crate("provider")
        .expect("fresh context should contain provider");
    let runtime_items = runtime_provider.metadata().language_items();
    let runtime_read = runtime_items
        .index
        .as_ref()
        .expect("loaded provider should expose ReadAt");
    let runtime_write = runtime_items
        .index_mut
        .as_ref()
        .expect("loaded provider should expose WriteAt");
    let remapped =
        |id: ProductDefId| DefId::new(runtime_provider.crate_id(), LocalDefId(id.local_id.0));

    assert_eq!(runtime_read.trait_id, remapped(provider_read.trait_id));
    assert_eq!(runtime_read.method_id, remapped(provider_read.method_id));
    assert_eq!(runtime_write.trait_id, remapped(provider_write.trait_id));
    assert_eq!(runtime_write.method_id, remapped(provider_write.method_id));

    let temp_entry = temp_dir.join("downstream.rk");
    fs::write(&temp_entry, downstream_source).unwrap();
    let output = crate::compile_with_products(&Config {
        entry_file: temp_entry,
        output_dir: temp_dir.join("downstream-build"),
        debug_print: vec![],
        meta_files: vec![],
        extern_artifacts: vec![("provider".to_string(), provider_artifact)],
        source_providers: Vec::new(),
        current_crate_name: Some("downstream".to_string()),
        opt_level: 0,
        overflow_checks: None,
        emit_llvm: false,
        no_link: true,
        emit_object: None,
        no_prelude: true,
        no_std: true,
        sysroot: None,
    })
    .expect("downstream source should compile from the provider artifact only");
    let products = output
        .products
        .expect("downstream products should be emitted");
    let probe = products
        .bodies
        .functions
        .values()
        .find(|function| function.name == "probe")
        .expect("downstream generic probe body should be present");
    let targets = collect_index_authorities_from_function(probe);
    assert_eq!(targets.len(), 2);

    let mut read_count = 0;
    let mut write_count = 0;
    for target in targets {
        let HirSelectedMethodTarget::ImplMethod {
            selected_trait: Some(selected_trait),
            ..
        } = target.target
        else {
            panic!("indexed call must preserve provider trait authority");
        };
        if selected_trait.trait_id == runtime_read.trait_id {
            read_count += 1;
            assert_eq!(selected_trait.member_id, runtime_read.method_id);
        } else if selected_trait.trait_id == runtime_write.trait_id {
            write_count += 1;
            assert_eq!(selected_trait.member_id, runtime_write.method_id);
        } else {
            panic!("indexed call selected an unrelated provider trait");
        }
    }
    assert_eq!((read_count, write_count), (1, 1));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn unmarked_try_and_index_protocol_spellings_have_no_authority_without_provider() {
    let try_diagnostics = compile_artifact_app_without_std_failure(
        r#"enum ControlFlow B, C
    Break B
    Continue C

trait Try
    type Output
    type Residual
    ~@branch: ControlFlow Self::Residual, Self::Output

trait FromResidual R
    from_residual: R -> Self

enum Maybe T
    Some T
    None

main = ->
    value = Maybe::Some 1
    value?
"#,
        Vec::new(),
    );
    assert!(try_diagnostics.0.iter().any(|diagnostic| {
        diagnostic.message == "Cannot use '?' because the Try language-item protocol is unavailable"
    }));

    let index_diagnostics = compile_artifact_app_without_std_failure(
        r#"trait Index Key
    type Output
    @index: Key -> &Self::Output

struct Boxed
    < value: I64

impl Index I64 for Boxed
    type Output = I64
    @index = _ -> &@value

main = ->
    boxed = Boxed value: 42
    boxed[0]
"#,
        Vec::new(),
    );
    assert!(index_diagnostics.0.iter().any(|diagnostic| {
        diagnostic.message
            == "Cannot use indexing because the Index language-item protocol is unavailable"
    }));
}

#[test]
fn duplicate_language_item_provider_artifacts_report_sorted_provider_names() {
    let temp_dir = temp_test_dir("duplicate_language_item_provider");
    let alpha_dir = temp_dir.join("alpha");
    let zeta_dir = temp_dir.join("zeta");
    let alpha_artifact = temp_dir.join("alpha.rkca");
    let zeta_artifact = temp_dir.join("zeta.rkca");
    write_crate(&alpha_dir, "alpha", "", "lang sized\n< trait AlphaShape\n");
    write_crate(&zeta_dir, "zeta", "", "lang sized\n< trait ZetaShape\n");
    build_product_artifact(&alpha_dir, "alpha", &alpha_artifact, true, true);
    build_product_artifact(&zeta_dir, "zeta", &zeta_artifact, true, true);

    let diagnostics = compile_artifact_app_without_std_failure(
        "main = -> 0\n",
        vec![
            ("zeta".to_string(), zeta_artifact),
            ("alpha".to_string(), alpha_artifact),
        ],
    );
    let messages = diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let conflict = messages
        .lines()
        .find(|message| message.contains("protocol 'sized'"))
        .unwrap_or_else(|| panic!("expected sized provider conflict, got: {messages}"));
    assert!(conflict.contains("alpha, zeta"));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn intermediate_artifact_does_not_reexport_provider_language_items() {
    let temp_dir = temp_test_dir("intermediate_language_item_non_reexport");
    let provider_dir = temp_dir.join("provider");
    let bridge_dir = temp_dir.join("bridge");
    let isolated_dir = temp_dir.join("isolated_bridge");
    let provider_artifact = temp_dir.join("provider.rkca");
    let bridge_artifact = temp_dir.join("bridge.rkca");
    let isolated_artifact = temp_dir.join("isolated_bridge.rkca");

    write_crate(
        &provider_dir,
        "provider",
        "",
        r#"lang index
< trait Slot Key
    lang output
    type Yield
    lang method
    @fetch: Key -> &Self::Yield

lang control_flow
< enum Fork B, C
    lang break
    Stop B
    lang continue
    Go C

lang try
< trait Passage
    lang output
    type Resulting
    lang residual
    type Debris
    lang branch
    ~@divide: Fork Self::Debris, Self::Resulting

lang from_residual
< trait Rebuild R
    lang method
    heal: R -> Self

struct Rack
    < value: I64
< Rack

impl Slot I64 for Rack
    type Yield = I64
    @fetch = _ -> &@value

enum Packet T
    Ready T
    Stalled
< Packet

enum Residue
    Stalled I64

impl Passage for Packet T
    type Resulting = T
    type Debris = Residue
    ~@divide = ->
        match self
            Packet::Ready value => Fork::Go value
            Packet::Stalled => Fork::Stop (Residue::Stalled 7)

impl Rebuild Residue for Packet T
    heal = residue ->
        match residue
            Residue::Stalled _ => Packet::Stalled
"#,
    );
    build_product_artifact(&provider_dir, "provider", &provider_artifact, true, true);

    write_crate(
        &bridge_dir,
        "bridge",
        "",
        r#"> provider::*

consume: Packet I64 -> Packet I64
consume = packet ->
    rack = Rack
        value: 5
    indexed = rack[0]
    value = packet?
    Packet::Ready value
< consume
"#,
    );
    build_product_artifact_with_extern_artifacts(
        &bridge_dir,
        "bridge",
        &bridge_artifact,
        true,
        true,
        vec![("provider".to_string(), provider_artifact.clone())],
    );

    let bridge_products = CompilerProducts::read_artifact_from_path(&bridge_artifact).unwrap();
    assert_eq!(
        bridge_products.interface.language_items,
        ProductLanguageItems::default(),
        "bridge must serialize only language items it provides"
    );
    assert_eq!(bridge_products.dependencies.len(), 1);
    assert_eq!(bridge_products.dependencies[0].name, "provider");

    let missing_dependency = CrateContext::new()
        .load_product_artifact_from_path_as("bridge", bridge_artifact.clone())
        .expect_err("bridge must retain its provider dependency integrity");
    assert!(
        missing_dependency.contains("dependency"),
        "expected missing dependency diagnostic, got: {missing_dependency}"
    );

    write_crate(
        &isolated_dir,
        "isolated_bridge",
        "",
        "bridge_value = -> 0\n< bridge_value\n",
    );
    build_product_artifact(
        &isolated_dir,
        "isolated_bridge",
        &isolated_artifact,
        true,
        true,
    );
    let isolated_products = CompilerProducts::read_artifact_from_path(&isolated_artifact).unwrap();
    assert_eq!(
        isolated_products.interface.language_items,
        ProductLanguageItems::default(),
        "isolated non-provider artifact must not confer protocol authority"
    );

    let diagnostics = compile_artifact_app_without_std_failure(
        r#"> isolated_bridge::*

enum ControlFlow B, C
    Break B
    Continue C
trait Try
    type Output
    type Residual
    ~@branch: ControlFlow Self::Residual, Self::Output
trait FromResidual R
    from_residual: R -> Self
enum Maybe T
    Some T
    None
main = ->
    value = Maybe::Some 1
    value?
"#,
        vec![("isolated_bridge".to_string(), isolated_artifact.clone())],
    );
    assert!(diagnostics.0.iter().any(|diagnostic| {
        diagnostic.message == "Cannot use '?' because the Try language-item protocol is unavailable"
    }));
    let index_diagnostics = compile_artifact_app_without_std_failure(
        r#"> isolated_bridge::*

trait Index Key
    type Output
    @index: Key -> &Self::Output
struct Boxed
    < value: I64
impl Index I64 for Boxed
    type Output = I64
    @index = _ -> &@value
main = ->
    boxed = Boxed value: 1
    boxed[0]
"#,
        vec![("isolated_bridge".to_string(), isolated_artifact)],
    );
    assert!(index_diagnostics.0.iter().any(|diagnostic| {
        diagnostic.message
            == "Cannot use indexing because the Index language-item protocol is unavailable"
    }));

    let (stdout, exit_code) = compile_and_run_artifact_app(
        r#"> provider::*
> bridge::consume

main: () -> I64
main = ->
    result = consume (Packet::Ready 7)
    0
"#,
        vec![
            ("provider".to_string(), provider_artifact),
            ("bridge".to_string(), bridge_artifact),
        ],
    );
    assert_eq!(stdout, "");
    assert_eq!(exit_code, 0);

    let _ = fs::remove_dir_all(temp_dir);
}

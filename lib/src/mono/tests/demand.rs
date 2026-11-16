use crate::{analyze, Config, CrateKind, SourceProvider};
use crate::crate_system::CrateContext;
use crate::mono::{monomorphize_with_crates, CompilationIdentityContext, InstanceOrigin, MonomorphizedProgram};

fn binary(source: &str) -> MonomorphizedProgram {
    let path = std::path::PathBuf::from("/virtual/mono-demand/main.rk");
    let analysis = analyze(&Config {
        entry_file: path.clone(),
        no_std: true,
        no_prelude: true,
        source_providers: vec![SourceProvider::Virtual { path, text: source.to_string() }],
        ..Config::default()
    }).unwrap();
    // `analyze` retains dead bodies: these tests exercise mono independently
    // of the earlier source-item pruning pass.
    monomorphize_with_crates(
        analysis.hir, &CrateContext::new(),
        CompilationIdentityContext::local_default(), CrateKind::Bin,
    ).unwrap()
}

#[test]
fn binary_worklist_does_not_process_unreferenced_bodies() {
    let output = binary(
        "dead = -> nested!\nnested = -> 1\nused = -> 42\nmain = -> used!\n"
    );
    let names = output.instances.values().map(|record| record.symbols.source_name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names.contains(&"main"));
    assert!(names.contains(&"used"));
}

#[test]
fn function_values_and_nested_closures_demand_their_callees() {
    let output = binary(
        "callback = -> 42\nmain = ->\n    f = callback\n    wrapper = (-> f!)\n    wrapper!\n"
    );
    assert_eq!(output.instances.len(), 2);
    assert!(output.instances.values().any(|record| record.symbols.source_name == "callback"));
}

#[test]
fn generic_recursion_interns_each_substitution_once() {
    let output = binary(
        "repeat = n, value ->\n    if ~I64Eq n, 0\n        value\n    else\n        repeat (~I64Sub n, 1), value\nmain = ->\n    repeat 2, true\n    repeat 3, 42\n"
    );
    let specializations = output.instances.values()
        .filter(|record| record.is_specialization).collect::<Vec<_>>();
    assert_eq!(specializations.len(), 2);
    assert_ne!(specializations[0].substitution, specializations[1].substitution);
    assert!(specializations.iter().all(|record| matches!(record.origin, InstanceOrigin::Function(_))));
}

#[test]
fn only_selected_trait_methods_are_materialized_for_binaries() {
    let output = binary(
        "unused_helper = value -> value\ntrait Value\n    @get: I64\n    @unused: I64\nstruct Box\n    < value: I64\nimpl Value for Box\n    @get = -> @value\n    @unused = -> unused_helper 99\nmain = ->\n    b = Box\n        value: 42\n    b.get!\n"
    );
    assert!(output.instances.values().any(|record| record.symbols.source_name.ends_with("::get")));
    assert!(!output.instances.values().any(|record| record.symbols.source_name.ends_with("::unused")));
    assert!(!output.instances.values().any(|record| record.symbols.source_name == "unused_helper"));
}

#[test]
fn demanded_instances_build_a_consistent_mir_backend_contract() {
    let mut output = binary(
        "helper = -> 42\nunused = -> 3\nmain = ->\n    f = helper\n    f!\n"
    );
    let bodies = crate::mir::builder::MirBuilder::take_mir_instance_bodies(&mut output).unwrap();
    let mir = crate::mir::builder::MirBuilder::build_monomorphized_with_instance_bodies(&output, &bodies).unwrap();
    assert!(crate::mir::agreement::check_mir_runtime_agreement(&mir).is_clean());
}

#[test]
fn generic_impl_static_members_are_not_library_instance_roots() {
    let path = std::path::PathBuf::from("/virtual/mono-demand/lib.rk");
    let analysis = analyze(&Config {
        entry_file: path.clone(),
        crate_kind: CrateKind::Lib,
        no_std: true,
        no_prelude: true,
        source_providers: vec![SourceProvider::Virtual {
            path,
            text: "< struct Phantom T\nimpl Phantom T\n    count = -> 42\n".to_string(),
        }],
        ..Config::default()
    }).unwrap();
    let output = monomorphize_with_crates(
        analysis.hir, &CrateContext::new(),
        CompilationIdentityContext::local_default(), CrateKind::Lib,
    ).unwrap();
    assert!(output.instances.is_empty());
}

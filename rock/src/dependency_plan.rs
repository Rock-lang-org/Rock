//! Dependency units distinguish execution platform from ordinary source target role.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use rock_shared::manifest::{CrateTarget, TargetRole};

use crate::deps::resolve_dependency_root;
use crate::package::Package;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BuildPlatform { Host, Target }

#[derive(Debug, Clone)]
pub(crate) struct BuildTriples {
    pub(crate) host: String,
    pub(crate) target: String,
}

impl BuildTriples {
    pub(crate) fn native() -> Self {
        let host = rock_shared::sysroot::host_target_triple();
        Self { target: host.clone(), host }
    }

    pub(crate) fn unit(&self, package: &Package, parent: BuildPlatform) -> BuildUnitId {
        let platform = if package.target.role() == TargetRole::ProcMacro { BuildPlatform::Host } else { parent };
        BuildUnitId { root: package.root_dir.clone(), target: package.target.clone(), platform,
            triple: match platform { BuildPlatform::Host => self.host.clone(), BuildPlatform::Target => self.target.clone() } }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct BuildUnitId {
    pub(crate) root: PathBuf,
    pub(crate) target: CrateTarget,
    pub(crate) platform: BuildPlatform,
    pub(crate) triple: String,
}

impl BuildUnitId {
    pub(crate) fn role(&self) -> TargetRole { self.target.role() }

    pub(crate) fn apply(&self, package: &mut Package, project_root: &std::path::Path) {
        package.target = self.target.clone(); package.platform = self.platform; package.target_triple = self.triple.clone();
        package.use_project_build(project_root);
    }
}

#[derive(Debug)]
pub(crate) struct PlannedUnit {
    pub(crate) id: BuildUnitId,
    pub(crate) package: Package,
    /// Metadata plus ordinary-library closures, stopping at every proc boundary.
    pub(crate) compiler_inputs: Vec<BuildUnitId>,
    pub(crate) runtime_inputs: Vec<BuildUnitId>,
}

#[derive(Debug)]
pub(crate) struct BuildPlan {
    pub(crate) root: BuildUnitId,
    pub(crate) units: HashMap<BuildUnitId, PlannedUnit>,
    pub(crate) order: Vec<BuildUnitId>,
}

impl BuildPlan {
    pub(crate) fn new(package: &Package, triples: BuildTriples) -> Result<Self, String> {
        Self::for_project(package, triples, &package.root_dir)
    }

    pub(crate) fn for_project(package: &Package, triples: BuildTriples, project_root: &std::path::Path) -> Result<Self, String> {
        Self::with_known_packages(package, triples, project_root, &HashMap::new())
    }

    pub(crate) fn with_known_packages(package: &Package, triples: BuildTriples, project_root: &std::path::Path, known: &HashMap<PathBuf, Package>) -> Result<Self, String> {
        let root = triples.unit(package, package.platform);
        let mut plan = Self { root: root.clone(), units: HashMap::new(), order: Vec::new() };
        plan.visit(package.clone(), root, &triples, project_root, &mut Vec::new(), known)?;
        Ok(plan)
    }

    fn visit(&mut self, mut package: Package, id: BuildUnitId, triples: &BuildTriples,
        project_root: &std::path::Path, stack: &mut Vec<BuildUnitId>, known: &HashMap<PathBuf, Package>) -> Result<(), String> {
        if self.units.contains_key(&id) { return Ok(()); }
        if let Some(start) = stack.iter().position(|active| *active == id) {
            let chain = stack[start..].iter().chain(std::iter::once(&id)).map(|unit|
                format!("{} [{} {:?} {}]", unit.root.display(), if unit.platform == BuildPlatform::Host { "host" } else { "target" }, unit.role(), unit.triple))
                .collect::<Vec<_>>().join(" -> ");
            return Err(format!("Circular dependency detected: {chain}"));
        }
        id.apply(&mut package, project_root);
        stack.push(id.clone());
        let mut compiler_inputs = Vec::new();
        let mut seen = HashSet::new();
        if let Some(dependencies) = &package.manifest.dependencies {
            for (name, dependency) in dependencies {
                let root = resolve_dependency_root(&package.root_dir, name, dependency)?;
                let mut child = match known.get(&root) {
                    Some(package) => package.clone(),
                    None => Package::load(root)?,
                };
                child.target = child.manifest.library().cloned().ok_or_else(|| format!(
                    "Package '{}' has no library target and cannot be used as a dependency; add a [lib] target", child.manifest.crate_.name))?;
                child.profile = package.profile; child.no_proc_execution = package.no_proc_execution;
                let child_id = triples.unit(&child, id.platform);
                self.visit(child, child_id.clone(), triples, project_root, stack, known)?;
                if seen.insert(child_id.clone()) { compiler_inputs.push(child_id.clone()); }
                if child_id.role() != TargetRole::ProcMacro {
                    for transitive in &self.units[&child_id].compiler_inputs {
                        if seen.insert(transitive.clone()) { compiler_inputs.push(transitive.clone()); }
                    }
                }
            }
        }
        let runtime_inputs = compiler_inputs.iter().filter(|input| input.role() != TargetRole::ProcMacro).cloned().collect();
        stack.pop(); self.order.push(id.clone());
        self.units.insert(id.clone(), PlannedUnit { id, package, compiler_inputs, runtime_inputs });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::support::{load_package, temp_test_dir, write_package_with_options};

    #[test]
    fn macro_b_15_host_closure_is_separate_from_target_inputs() {
        let root = temp_test_dir("macro_b15_units");
        let app = root.join("app"); let provider = root.join("provider");
        let nested = root.join("nested"); let sdk = root.join("sdk");
        let common = root.join("common"); let std = root.join("std");
        for (path, name, deps) in [
            (&std, "std", vec![]), (&common, "common", vec![]),
            (&sdk, "chosen_sdk", vec![("std", std.display().to_string())]),
            (&nested, "nested", vec![("chosen_sdk", sdk.display().to_string())]),
            (&provider, "provider", vec![("chosen_sdk", sdk.display().to_string()), ("nested", nested.display().to_string()), ("common", common.display().to_string())]),
            (&app, "app", vec![("provider", provider.display().to_string()), ("common", common.display().to_string())]),
        ] {
            write_package_with_options(path, name, if name == "app" { "src/main.rk" } else { "lib.rk" }, &deps, "not parsed by planning", true);
        }
        for path in [&provider, &nested] {
            let manifest = path.join("rock.toml");
            let text = std::fs::read_to_string(&manifest).unwrap().replace("[lib]", "[lib]\nproc_macro = true");
            std::fs::write(manifest, text).unwrap();
        }
        let package = load_package(app.clone());
        let plan = BuildPlan::new(&package, BuildTriples { host: "host-fixture".into(), target: "target-fixture".into() }).unwrap();
        let target_common = plan.units.values().find(|unit| unit.package.root_dir == common && unit.id.platform == BuildPlatform::Target).unwrap();
        let host_common = plan.units.values().find(|unit| unit.package.root_dir == common && unit.id.platform == BuildPlatform::Host).unwrap();
        assert_ne!(host_common.id, target_common.id);
        assert_ne!(host_common.package.artifact_path(), target_common.package.artifact_path());
        assert_eq!(host_common.id.triple, "host-fixture"); assert_eq!(target_common.id.triple, "target-fixture");
        let consumer = &plan.units[&plan.root];
        assert_eq!(consumer.compiler_inputs.len(), 2, "proc SDK/std/nested host dependencies are not consumer inputs");
        assert_eq!(consumer.runtime_inputs.len(), 1);
        assert_eq!(consumer.runtime_inputs[0], target_common.id);
        for unit in plan.units.values().filter(|unit| unit.package.root_dir != app && unit.package.root_dir != common) {
            assert_eq!(unit.id.platform, BuildPlatform::Host);
        }
        let provider = plan.units.values().find(|unit| unit.package.root_dir == provider).unwrap();
        assert_eq!(provider.compiler_inputs.len(), 4);
        assert!(provider.runtime_inputs.iter().all(|id| id.platform == BuildPlatform::Host && id.role() == rock_shared::manifest::TargetRole::Library));
        let sdk_position = plan.order.iter().position(|id| id.root == sdk).unwrap();
        let nested_position = plan.order.iter().position(|id| id.root == nested).unwrap();
        let provider_position = plan.order.iter().position(|id| *id == provider.id).unwrap();
        assert!(sdk_position < nested_position && nested_position < provider_position);
        let native = BuildPlan::new(&package, BuildTriples::native()).unwrap();
        let mut common_units = native.units.values().filter(|unit| unit.package.root_dir == common);
        let first = common_units.next().unwrap(); let second = common_units.next().unwrap();
        assert_eq!(first.id.triple, second.id.triple);
        assert_ne!(first.package.artifact_path(), second.package.artifact_path(), "native host/target artifacts are not shared");
        assert_ne!(first.package.object_path(), second.package.object_path());
        let mut ordinary = provider.package.clone(); ordinary.target = CrateTarget::Lib { path: ordinary.target.path().into() };
        let ordinary_id = BuildTriples::native().unit(&ordinary, BuildPlatform::Host);
        ordinary_id.apply(&mut ordinary, &app);
        assert_ne!(ordinary.artifact_path(), native.units.values().find(|unit| unit.package.root_dir == provider.package.root_dir).unwrap().package.artifact_path(), "proc and ordinary roles cannot share artifacts even for the same entry");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn macro_b_15_cycles_report_host_dependency_chain_without_compiling() {
        let root = temp_test_dir("macro_b15_cycle"); let a = root.join("a"); let b = root.join("b");
        write_package_with_options(&a, "a", "lib.rk", &[("b", b.display().to_string())], "not parsed", true);
        write_package_with_options(&b, "b", "lib.rk", &[("a", a.display().to_string())], "not parsed", true);
        let manifest = a.join("rock.toml");
        std::fs::write(&manifest, std::fs::read_to_string(&manifest).unwrap().replace("[lib]", "[lib]\nproc_macro = true")).unwrap();
        let error = BuildPlan::new(&load_package(a.clone()), BuildTriples::native()).unwrap_err();
        assert!(error.contains("Circular dependency") && error.contains("host") && error.contains("a") && error.contains("b"), "{error}");
        assert!(!a.join("build").exists() && !b.join("build").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}

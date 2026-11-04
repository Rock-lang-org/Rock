use std::collections::HashMap;

use super::PreludeImports;
use crate::collect::resolver::ResolverTables;
use crate::crate_artifact::ArtifactExport;
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::lower::items::LowerItems;
use crate::lower::scope::Scope;

#[test]
fn inject_loaded_prelude_reports_missing_declarations_by_short_name() {
    let prelude = PreludeImports::with_exports(
        true,
        HashMap::from([
            (
                "zebra".to_string(),
                ArtifactExport {
                    source: "std::zebra".to_string(),
                    id: DefId::new(CrateId(7), LocalDefId(5)),
                },
            ),
            (
                "omega".to_string(),
                ArtifactExport {
                    source: "std::omega".to_string(),
                    id: DefId::new(CrateId(7), LocalDefId(4)),
                },
            ),
            (
                "gamma".to_string(),
                ArtifactExport {
                    source: "std::gamma".to_string(),
                    id: DefId::new(CrateId(7), LocalDefId(3)),
                },
            ),
            (
                "beta".to_string(),
                ArtifactExport {
                    source: "std::beta".to_string(),
                    id: DefId::new(CrateId(7), LocalDefId(2)),
                },
            ),
            (
                "alpha".to_string(),
                ArtifactExport {
                    source: "std::alpha".to_string(),
                    id: DefId::new(CrateId(7), LocalDefId(1)),
                },
            ),
        ]),
    );

    let errors = prelude.inject_loaded_prelude(
        &mut Scope::new(),
        &mut LowerItems::new(),
        &mut ResolverTables::default(),
    );

    assert_eq!(
        errors,
        vec![
            "Missing loaded prelude declaration for std::alpha",
            "Missing loaded prelude declaration for std::beta",
            "Missing loaded prelude declaration for std::gamma",
            "Missing loaded prelude declaration for std::omega",
            "Missing loaded prelude declaration for std::zebra",
        ]
    );
}

impl PreludeImports {
    pub(crate) fn export_source(&self, short_name: &str) -> Option<&str> {
        self.loaded_exports
            .get(short_name)
            .map(|export| export.source.as_str())
    }

    pub(crate) fn export_short_name_is_user_owned(
        resolver: &ResolverTables,
        short_name: &str,
        export_id: DefId,
    ) -> bool {
        resolver.item_paths.contains_key(short_name)
            || resolver
                .import_aliases
                .get(short_name)
                .is_some_and(|alias_id| *alias_id != export_id)
    }
}

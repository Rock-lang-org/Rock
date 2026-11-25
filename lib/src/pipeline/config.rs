use std::path::PathBuf;

use crate::lint::LintLevels;
use crate::target::Target;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DebugPrint {
    Ast,
    AstFull,
    Expanded,
    Tokens,
    Hir,
    Mir,
    Llvm,
}

impl From<&str> for DebugPrint {
    fn from(s: &str) -> Self {
        match s {
            "ast" => DebugPrint::Ast,
            "ast-full" => DebugPrint::AstFull,
            "expanded" => DebugPrint::Expanded,
            "tokens" => DebugPrint::Tokens,
            "hir" => DebugPrint::Hir,
            "mir" => DebugPrint::Mir,
            "llvm" => DebugPrint::Llvm,
            _ => panic!(
                "Unknown debug print: {}\nValid options are: ast, ast-full, expanded, tokens, hir, mir, llvm",
                s
            ),
        }
    }
}

/// What the crate is built as; decides which items are roots for
/// reachability (a bin's exports are crate-internal only).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum CrateKind {
    #[default]
    Bin,
    Lib,
}

#[derive(Debug, Default, Clone)]
pub struct Config {
    pub crate_kind: CrateKind,
    pub lint_levels: LintLevels,
    pub entry_file: PathBuf,
    pub output_dir: PathBuf,
    pub debug_print: Vec<DebugPrint>,
    pub meta_files: Vec<(String, PathBuf)>, // crate name, path
    pub extern_artifacts: Vec<(String, PathBuf)>, // External product artifacts: name=path
    pub source_providers: Vec<SourceProvider>, // Explicit non-filesystem source inputs
    pub current_crate_name: Option<String>,
    pub opt_level: u8, // 0-3
    /// None selects development checks / release wrapping.
    pub overflow_checks: Option<bool>,
    pub emit_llvm: bool,
    pub no_link: bool,
    pub emit_object: Option<PathBuf>,
    pub no_prelude: bool,         // Don't inject std prelude
    pub no_std: bool,             // Don't auto-load bundled std
    pub sysroot: Option<PathBuf>, // Toolchain sysroot override
    /// Target triple; `None` builds for the host.
    pub target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceProvider {
    Virtual {
        path: PathBuf,
        text: String,
    },
    Artifact {
        path: PathBuf,
        artifact_path: PathBuf,
        text: String,
    },
}

impl Config {
    pub fn has_debug_print(&self, name: DebugPrint) -> bool {
        self.debug_print.contains(&name)
    }

    /// The resolved compilation target. CLIs validate explicit triples before
    /// building a config, so an invalid one here falls back to the host.
    pub fn target(&self) -> Target {
        self.target
            .as_deref()
            .and_then(|triple| Target::parse(triple).ok())
            .unwrap_or_else(Target::host)
    }
}

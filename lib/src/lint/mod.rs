//! Lint identities, levels and the sink that turns lint findings into
//! warnings (or errors, for denied lints).
//!
//! Lint passes report through [`LintSink::emit`]; the configured
//! [`LintLevels`] decide whether a finding is dropped, warned or denied.

mod unused_imports;
mod unused_variables;
pub(crate) mod walk;

pub use unused_imports::SourceModules;

use crate::diagnostic::{Diagnostic, DiagnosticCode, DiagnosticSeverity, Diagnostics};
use crate::infer::ResolvedHirProgram;

/// Runs every lint pass over the current crate: its expanded source modules
/// and its finalized HIR.
pub fn run_lints(
    modules: &SourceModules<'_>,
    hir: &ResolvedHirProgram,
    levels: &LintLevels,
) -> Diagnostics {
    let mut sink = LintSink::new(levels);
    unused_imports::check_unused_imports(modules, &mut sink);
    unused_variables::check_unused_variables(hir, &mut sink);
    sink.finish()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Lint {
    UnusedImports,
    UnusedVariables,
    DeadCode,
}

impl Lint {
    pub const ALL: [Lint; 3] = [Lint::UnusedImports, Lint::UnusedVariables, Lint::DeadCode];

    pub const fn name(self) -> &'static str {
        match self {
            Lint::UnusedImports => "unused_imports",
            Lint::UnusedVariables => "unused_variables",
            Lint::DeadCode => "dead_code",
        }
    }

    pub const fn default_level(self) -> LintLevel {
        LintLevel::Warn
    }

    /// Groups this lint belongs to, besides its own name.
    const fn groups(self) -> &'static [&'static str] {
        &["unused"]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LintLevel {
    Allow,
    Warn,
    Deny,
}

/// Effective level of every lint, built from defaults plus ordered
/// command-line overrides (later overrides win, like rustc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LintLevels {
    levels: [LintLevel; Lint::ALL.len()],
}

impl Default for LintLevels {
    fn default() -> Self {
        Self {
            levels: Lint::ALL.map(Lint::default_level),
        }
    }
}

impl LintLevels {
    pub fn level(&self, lint: Lint) -> LintLevel {
        self.levels[lint as usize]
    }

    /// Sets `level` for a lint or lint group name (`unused`, `dead_code`, ...).
    pub fn set(&mut self, name: &str, level: LintLevel) -> Result<(), String> {
        let mut matched = false;
        for lint in Lint::ALL {
            if lint.name() == name || lint.groups().contains(&name) {
                self.levels[lint as usize] = level;
                matched = true;
            }
        }
        if matched {
            Ok(())
        } else {
            Err(format!("unknown lint: `{name}`"))
        }
    }

    pub fn is_allowed(&self, lint: Lint) -> bool {
        self.level(lint) == LintLevel::Allow
    }
}

/// Collects lint findings at their configured level.
pub struct LintSink<'a> {
    levels: &'a LintLevels,
    diagnostics: Diagnostics,
}

impl<'a> LintSink<'a> {
    pub fn new(levels: &'a LintLevels) -> Self {
        Self {
            levels,
            diagnostics: Diagnostics::default(),
        }
    }

    pub fn is_enabled(&self, lint: Lint) -> bool {
        !self.levels.is_allowed(lint)
    }

    /// Records `diagnostic` for `lint`, unless the lint is allowed.
    pub fn emit(&mut self, lint: Lint, diagnostic: Diagnostic) {
        let (severity, note) = match self.levels.level(lint) {
            LintLevel::Allow => return,
            LintLevel::Warn => (DiagnosticSeverity::Warning, "on by default"),
            LintLevel::Deny => (DiagnosticSeverity::Error, "denied"),
        };
        let note = format!("`{}` {note}", lint.name());
        self.diagnostics.0.push(
            diagnostic
                .with_code(DiagnosticCode::Lint)
                .with_severity(severity)
                .with_note(note),
        );
    }

    /// The findings in source order, independent of pass and map iteration order.
    pub fn finish(mut self) -> Diagnostics {
        self.diagnostics.0.sort_by(|left, right| {
            let key = |diagnostic: &Diagnostic| {
                diagnostic
                    .primary
                    .as_ref()
                    .map(|label| (label.span.file_path.clone(), label.span.start, label.span.end))
            };
            key(left).cmp(&key(right))
        });
        self.diagnostics
    }
}

/// True when a binding or item name opts out of unused lints (`_` prefix).
pub fn is_intentionally_unused(name: &str) -> bool {
    name.starts_with('_')
}

#[cfg(test)]
mod tests;

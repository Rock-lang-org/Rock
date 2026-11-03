use std::fs;

use rock_lib::{
    ast::{
        visit::{walk_module, Visitor},
        Module, Program,
    },
    diagnostic::Diagnostics,
};

use super::{diagnostics_from_message, Config};

pub(super) fn load_program_for_source_command(
    rockc_config: &rock_lib::Config,
) -> Result<Program, Diagnostics> {
    load_program_with_source_database(rockc_config).map(|(program, _)| program)
}

pub(super) fn load_program_with_source_database(
    rockc_config: &rock_lib::Config,
) -> Result<(Program, rock_lib::source_loader::SourceDatabase), Diagnostics> {
    let mut source_db = rock_lib::source_loader::SourceDatabase::new();
    for provider in &rockc_config.source_providers {
        source_db.add_source_provider(provider);
    }
    let graph = source_db
        .load_entry(rockc_config.entry_file.clone(), rockc_config)
        .map_err(rock_lib::source_loader::source_load_errors_to_diagnostics)?;
    Ok((
        Program {
            module: graph.root_module().clone(),
        },
        source_db,
    ))
}

pub(super) fn format_config(config: Config) -> Result<(), Diagnostics> {
    let entry_file = config
        .entry_file
        .ok_or_else(|| diagnostics_from_message("Missing required --entry-file for --format"))?;
    let rockc_config = rock_lib::Config {
        entry_file,
        ..rock_lib::Config::default()
    };

    let program = load_program_for_source_command(&rockc_config)?;
    let mut formatter = AstFormatter { result: Ok(()) };
    program.visit(&mut formatter);
    formatter.result
}

pub(super) fn expand_config(config: Config) -> Result<(), Diagnostics> {
    let entry_file = config
        .entry_file
        .ok_or_else(|| diagnostics_from_message("Missing required --entry-file for --expand"))?;
    let rockc_config = rock_lib::Config {
        entry_file,
        ..rock_lib::Config::default()
    };

    let (program, source_db) = load_program_with_source_database(&rockc_config)?;
    let diagnostic_sources =
        rock_lib::diagnostic::DiagnosticSourceMap::from_source_database(&source_db);
    let macro_context = rock_lib::macro_expansion::MacroExpansionContext::new(&rockc_config);
    let expanded = rock_lib::macro_expansion::expand_macros_with_context(program, &macro_context)
        .map_err(|diagnostics| diagnostics.with_sources(&diagnostic_sources))?;
    expanded.visit(&mut ModulePrinter);
    Ok(())
}

struct AstFormatter {
    result: Result<(), Diagnostics>,
}

impl<'a> Visitor<'a> for AstFormatter {
    fn visit_module(&mut self, module: &'a Module) {
        if self.result.is_err() {
            return;
        }

        let formatted = if let Some(path) = &module.filepath {
            let source = match fs::read_to_string(path) {
                Ok(source) => source,
                Err(err) => {
                    self.result = Err(diagnostics_from_message(format!(
                        "Failed to read source module {} for formatting: {}",
                        path.display(),
                        err
                    )));
                    return;
                }
            };

            rock_lib::fmt::format(rock_lib::fmt::FormatInput::module_with_source(
                module, &source,
            ))
        } else {
            rock_lib::fmt::format(rock_lib::fmt::FormatInput::module(module))
        };

        if let Some(path) = &module.filepath {
            if let Err(err) = fs::write(path, &formatted) {
                self.result = Err(diagnostics_from_message(format!(
                    "Failed to write formatted module {}: {}",
                    path.display(),
                    err
                )));
                return;
            }
        }

        println!("{}", formatted);
        walk_module(self, module);
    }
}

struct ModulePrinter;

impl<'a> Visitor<'a> for ModulePrinter {
    fn visit_module(&mut self, module: &'a Module) {
        if let Some(name) = &module.name {
            println!("### {}: ###\n", name.name);
        }

        println!(
            "{}",
            rock_lib::fmt::format(rock_lib::fmt::FormatInput::module(module))
        );
        walk_module(self, module);
    }
}

pub use crate::lexer::Span;

pub mod analysis;
pub mod ast;
pub mod codegen;
pub mod collect;
pub mod crate_artifact;
pub mod crate_system;
mod dead_code;
pub mod diagnostic;
pub mod fmt;
pub mod hir;
pub mod ids;
pub mod infer;
pub mod language_items;
mod lexer;
pub mod lint;
pub mod lower;
pub mod macro_expansion;
pub mod mir;
pub mod mono;
pub mod parser;
mod pipeline;
pub mod products;
pub mod selection;
pub mod source_loader;
pub mod source_map;
pub mod sysroot;
pub mod target;
pub mod traits;
pub mod type_context;
pub(crate) mod type_lowering;
pub mod type_services;
pub mod types;

pub use pipeline::{
    analyze, check, compile, compile_with_products, compile_with_warnings, CompileOutput, Config,
    CrateKind, DebugPrint, SourceProvider,
};
pub use source_loader::source_load_errors_to_diagnostics;

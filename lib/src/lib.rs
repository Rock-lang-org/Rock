pub use crate::lexer::Span;

pub mod analysis;
pub mod ast;
pub mod codegen;
pub mod collect;
pub mod crate_artifact;
pub mod crate_system;
pub mod dce;
pub mod diagnostic;
pub mod fmt;
pub mod hir;
pub mod ids;
pub mod infer;
pub mod language_items;
mod lexer;
pub mod lower;
pub mod macro_expansion;
pub mod mir;
pub mod mono;
pub mod parser;
mod pipeline;
pub mod products;
pub mod selection;
#[cfg(test)]
mod semantic_identity_audit;
pub mod source_loader;
pub mod source_map;
pub mod sysroot;
pub mod traits;
pub mod type_context;
pub(crate) mod type_lowering;
pub mod type_services;
pub mod types;

pub use pipeline::{
    analyze, compile, compile_with_products, CompileOutput, Config, DebugPrint, SourceProvider,
};
pub use source_loader::source_load_errors_to_diagnostics;

use crate::ast::Module;
use crate::diagnostic::Diagnostics;
use crate::macro_expansion::TokenStream;
use crate::{parser, Config};

pub fn parse_generated_module(
    stream: &TokenStream,
    config: &Config,
) -> Result<Module, Diagnostics> {
    parser::parse_module_tokens(&stream.to_tokens(), config).map_err(Diagnostics::from)
}

#[cfg(test)]
mod tests;

mod functions;
mod generic_names;
mod impls;
mod nominals;
mod traits;
mod where_clauses;

pub(crate) use functions::{
    build_function_header_with_id, build_function_header_with_sig, build_function_sig_with_id,
};
pub(crate) use impls::build_impl_with_id;
pub(crate) use nominals::{
    build_enum_with_id, build_extern_with_id, build_nominal_generic_params, build_struct_with_id,
    build_type_alias_with_id,
};
pub(crate) use traits::build_trait_with_id;

#[cfg(test)]
mod tests;

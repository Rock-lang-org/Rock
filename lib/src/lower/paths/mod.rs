//! Path, instance, lambda, and tuple lowering

mod constructor_members;
mod escaping_lambdas;
mod identifier_path;
mod instance;
mod lambda_captures;
mod lambdas;
mod static_bound_paths;
mod static_method_values;
mod static_owner_matching;

#[cfg(test)]
mod tests;

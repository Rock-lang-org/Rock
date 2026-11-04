use crate::ast;
use crate::collect::context::path_names;
use std::collections::HashMap;

/// Resolve descriptor dependencies first, then nominal fields. Field cycles do
/// not imply default-expansion cycles and must not order descriptor publication.
pub(super) fn ordered_headers(module: &ast::Module) -> Vec<(usize, &ast::TopLevel)> {
    use crate::ast::visit::Visitor;
    struct Names(Vec<String>);
    impl<'a> Visitor<'a> for Names {
        fn visit_parse_type_inner(&mut self, inner: &'a ast::ParseTypeInner) {
            self.0.push(inner.name.clone());
            crate::ast::visit::walk_parse_type_inner(self, inner);
        }
    }
    let owners = module
        .top_levels
        .iter()
        .enumerate()
        .filter_map(|(index, item)| match item {
            ast::TopLevel::StructDecl(_)
            | ast::TopLevel::EnumDecl(_)
            | ast::TopLevel::TraitDecl(_)
            | ast::TopLevel::NewType(_, _, _) => item.get_ident().map(|name| (name.clone(), index)),
            ast::TopLevel::Module(declaration) => declaration
                .0
                .name
                .as_ref()
                .map(|name| (name.name.clone(), index)),
            ast::TopLevel::Mod(name, _) => Some((name.name.clone(), index)),
            ast::TopLevel::Import(path) => {
                let segments = path.segments();
                path_names(segments)
                    .last()
                    .filter(|name| name.starts_with(char::is_uppercase))
                    .cloned()
                    .map(|name| (name, index))
            }
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    let dependencies = module
        .top_levels
        .iter()
        .map(|item| {
            let mut names = Names(Vec::new());
            match item {
                ast::TopLevel::StructDecl(decl) => {
                    for param in &decl.generic_params {
                        names.visit_generic_param_decl(param);
                    }
                }
                ast::TopLevel::EnumDecl(decl) => {
                    for param in &decl.generic_params {
                        names.visit_generic_param_decl(param);
                    }
                }
                _ => names.visit_top_level(item),
            }
            match item {
                ast::TopLevel::Import(path) => {
                    let segments = path.segments();
                    names.0.push(path_names(segments).join("::"));
                }
                ast::TopLevel::GlobImport(path) => names.0.push(path.join("::")),
                _ => {}
            }
            names
                .0
                .into_iter()
                .filter_map(|name| {
                    owners
                        .get(&name)
                        .or_else(|| {
                            name.split_once("::")
                                .and_then(|(module, _)| owners.get(module))
                        })
                        .copied()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    fn visit(index: usize, deps: &[Vec<usize>], state: &mut [u8], order: &mut Vec<usize>) {
        if state[index] != 0 {
            return;
        }
        state[index] = 1;
        for dependency in &deps[index] {
            visit(*dependency, deps, state, order);
        }
        state[index] = 2;
        order.push(index);
    }
    let mut order = Vec::new();
    let mut state = vec![0; module.top_levels.len()];
    let mut types = owners.values().copied().collect::<Vec<_>>();
    types.sort_unstable();
    for index in types {
        visit(index, &dependencies, &mut state, &mut order);
    }
    for index in 0..module.top_levels.len() {
        visit(index, &dependencies, &mut state, &mut order);
    }
    order
        .into_iter()
        .map(|index| (index, &module.top_levels[index]))
        .collect()
}

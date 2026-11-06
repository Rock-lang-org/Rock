use super::LocalCollector;
use crate::ast;
use crate::collect::headers;
use crate::hir::{HirEnum, HirStruct};
use crate::ids::DefId;
use std::path::PathBuf;

pub(super) enum NominalDeclaration {
    Struct(ast::StructDecl),
    Enum(ast::EnumDecl),
}

pub(super) struct PendingNominalFields {
    declaration: NominalDeclaration,
    module_path: PathBuf,
}

impl LocalCollector {
    pub(super) fn collect_struct_header(&mut self, sd: &ast::StructDecl, id: DefId) -> HirStruct {
        self.pending_nominal_fields.insert(
            id,
            PendingNominalFields {
                declaration: NominalDeclaration::Struct(sd.clone()),
                module_path: self.context.modules.current_path().to_path_buf(),
            },
        );
        HirStruct {
            id,
            name: sd.name.name.clone(),
            generic_params: headers::build_nominal_generic_params(
                &mut self.context,
                &sd.generic_params,
                id,
            ),
            fields: Vec::new(),
        }
    }

    pub(super) fn collect_enum_header(&mut self, ed: &ast::EnumDecl, id: DefId) -> HirEnum {
        self.pending_nominal_fields.insert(
            id,
            PendingNominalFields {
                declaration: NominalDeclaration::Enum(ed.clone()),
                module_path: self.context.modules.current_path().to_path_buf(),
            },
        );
        HirEnum {
            id,
            name: ed.name.name.clone(),
            generic_params: headers::build_nominal_generic_params(
                &mut self.context,
                &ed.generic_params,
                id,
            ),
            variants: Vec::new(),
        }
    }

    pub(super) fn complete_nominal_fields(&mut self) {
        // All source modules now expose completed defaults. Keep declaration
        // scope while filling fields, including mutually recursive file modules.
        for (id, pending) in std::mem::take(&mut self.pending_nominal_fields) {
            let outer = self.context.modules.enter(pending.module_path);
            match pending.declaration {
                NominalDeclaration::Struct(sd) => {
                    let params = self
                        .context
                        .structs
                        .values()
                        .find(|item| item.id == id)
                        .expect("nominal descriptors precede fields")
                        .generic_params
                        .clone();
                    let strukt = headers::build_struct_with_id(&mut self.context, &sd, id, params);
                    for alias in self
                        .context
                        .structs
                        .values_mut()
                        .filter(|item| item.id == id)
                    {
                        alias.clone_from(&strukt);
                    }
                }
                NominalDeclaration::Enum(ed) => {
                    let params = self
                        .context
                        .enums
                        .values()
                        .find(|item| item.id == id)
                        .expect("nominal descriptors precede fields")
                        .generic_params
                        .clone();
                    let enum_ = headers::build_enum_with_id(&mut self.context, &ed, id, params);
                    for alias in self.context.enums.values_mut().filter(|item| item.id == id) {
                        alias.clone_from(&enum_);
                    }
                }
            }
            self.context.modules.leave(outer);
        }
    }
}

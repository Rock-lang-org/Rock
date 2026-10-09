//! Compiler-owned native metadata runtime; no stdlib or compiler-host cache.
use std::path::{Path, PathBuf};

use inkwell::targets::{CodeModel, InitializationConfig, RelocMode, Target, TargetMachine};
use inkwell::OptimizationLevel;

use super::{CodeGen, CodegenError};

pub(crate) const HEADER: &str = include_str!("descriptor_runtime.h");
pub(crate) const SOURCE: &str = include_str!("descriptor_runtime.c");

/// Materialize the embedded component for conditional native linking and the
/// protocol harness; this function supplies no semantic or borrow authority.
pub(crate) fn write_sources(directory: &Path) -> std::io::Result<PathBuf> {
    std::fs::write(directory.join("rock_descriptor_runtime.h"), HEADER)?;
    let source = directory.join("rock_descriptor_runtime.c");
    std::fs::write(&source, SOURCE)?;
    Ok(source)
}

impl<'ctx> CodeGen<'ctx> {
    pub(crate) fn runtime_support(&self) -> Result<std::collections::BTreeSet<crate::products::ProductRuntimeSupport>, CodegenError> {
        let mut support = std::collections::BTreeSet::new();
        if self.dictionary_runtime_required {
            support.insert(crate::products::ProductRuntimeSupport::selected_dictionaries(self.descriptor_target_key()?));
        }
        if self.descriptor_runtime_required {
            support.insert(crate::products::ProductRuntimeSupport::tuple_descriptors(self.descriptor_target_key()?));
        }
        if self.descriptor_nominal_runtime_required {
            support.insert(crate::products::ProductRuntimeSupport::nominal_descriptors(self.descriptor_target_key()?));
        }
        Ok(support)
    }
    pub(super) fn descriptor_target_key(&self) -> Result<Vec<u8>, CodegenError> {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| Target::initialize_all(&InitializationConfig::default()));
        let triple = TargetMachine::get_default_triple();
        let target = Target::from_triple(&triple).map_err(|e| CodegenError::toolchain(e.to_string()))?;
        let machine = target.create_target_machine(&triple, "generic", "", OptimizationLevel::None,
            RelocMode::Default, CodeModel::Default).ok_or_else(|| CodegenError::toolchain("missing native metadata target"))?;
        let data = machine.get_target_data();
        let layout = data.get_data_layout();
        let triple_bytes = triple.as_str().to_bytes();
        let layout_bytes = layout.as_str().to_bytes();
        let native_index_64 = layout_bytes.split(|byte| *byte == b'-')
            .filter(|component| component.starts_with(b"p:") || component.starts_with(b"p0:"))
            .all(|component| {
                let fields = component.split(|byte| *byte == b':').collect::<Vec<_>>();
                fields.get(1).copied() == Some(b"64".as_slice())
                    && fields.get(4).map_or(true, |bits| *bits == b"64")
            });
        if data.get_pointer_byte_size(None) != 8 || data.get_abi_alignment(&self.context.i64_type()) != 8
            || data.get_abi_alignment(&self.context.ptr_type(inkwell::AddressSpace::default())) != 8 || !native_index_64
            || !layout_bytes.starts_with(b"e") || !triple_bytes.windows(5).any(|w| w == b"linux") {
            return Err(CodegenError::backend_contract("runtime descriptors currently require native little-endian 64-bit Linux with 8-byte i64 ABI alignment"));
        }
        // The layout comes from the same native LLVM machine used for output,
        // not guessed from a Rock type size or the compiler host's Rust layout.
        self.module.set_triple(&triple);
        self.module.set_data_layout(&layout);
        let mut key = b"RT01".to_vec();
        for bytes in [triple_bytes, layout_bytes] {
            key.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
            key.extend_from_slice(bytes);
        }
        Ok(key)
    }
}

#[cfg(test)]
#[path = "descriptor_runtime_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "descriptor_nominal_tests.rs"]
mod nominal_tests;

#[cfg(test)]
#[path = "descriptor_dictionary_tests.rs"]
mod dictionary_tests;

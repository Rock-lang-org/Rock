use std::collections::BTreeMap;

use inkwell::module::Linkage;
use inkwell::values::BasicMetadataValueEnum;
use inkwell::AddressSpace;

use crate::codegen::{CodeGen, CodegenError};
use crate::mir::{nominal_factory, MirBackendContract, MirErasedType, MirNominalDropAuthority};

impl<'ctx> CodeGen<'ctx> {
    pub(super) fn emit_nominal_destructors(&mut self, contract: &MirBackendContract) -> Result<(), CodegenError> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        let word = self.context.i64_type();
        for (owner, authority) in &contract.nominal_drop_authority {
            let MirNominalDropAuthority::Selected(selection) = authority else { continue; };
            let name = format!("rock.nominal.userdrop.{}.{}", owner.crate_id.0, owner.local.0);
            let key = nominal_factory::nominal_drop_template_key(contract, self.type_context(), selection)
                .map_err(|error| Self::erased_error(format!("{error:?}")))?;
            self.descriptor_bytes(&key, &format!("{name}.key"));
            let function = self.module.add_function(&name, self.context.void_type().fn_type(&[ptr.into(), ptr.into()], false), Some(Linkage::Internal));
            self.builder.position_at_end(self.context.append_basic_block(function, "entry"));
            let payload = function.get_nth_param(0).ok_or_else(|| Self::erased_error("nominal drop payload missing"))?.into_pointer_value();
            let descriptor = function.get_nth_param(1).ok_or_else(|| Self::erased_error("nominal drop descriptor missing"))?.into_pointer_value();
            let arguments = self.erased_descriptor_pointer(descriptor, 19)?;
            let mut layouts = BTreeMap::new();
            let crate::mir::MirNominalLayout::Struct { generic_params, .. } = &contract.nominal_layouts[owner] else {
                return Err(Self::erased_error("nominal destructor declaration layout missing"));
            };
            for index in 0..generic_params.len() {
                let slot = unsafe { self.builder.build_gep(ptr, arguments, &[word.const_int(index as u64, false)], "nominal.drop.argument.slot") }
                    .map_err(|error| Self::erased_error(error.to_string()))?;
                let value = self.builder.build_load(ptr, slot, "nominal.drop.argument")
                    .map_err(|error| Self::erased_error(error.to_string()))?.into_pointer_value();
                layouts.insert(MirErasedType::Parameter(index as u32), value);
            }
            let entry = &contract.erased.functions[&selection.entry];
            let subject = entry.signature.params[0].substitute(&selection.arguments)
                .map_err(|error| Self::erased_error(format!("{error:?}")))?;
            layouts.insert(subject, descriptor);
            let out = self.builder.build_alloca(word, "nominal.drop.unit")
                .map_err(|error| Self::erased_error(error.to_string()))?;
            let mut values: Vec<BasicMetadataValueEnum> = vec![out.into()];
            for layout in &entry.signature.layouts {
                let layout = layout.substitute(&selection.arguments).map_err(|error| Self::erased_error(format!("{error:?}")))?;
                values.push(self.erased_layout(&layout, &layouts)?.into());
            }
            if !entry.signature.dictionaries.is_empty() { return Err(Self::erased_error("nominal Drop requires unavailable dictionaries")); }
            values.push(payload.into());
            self.builder.build_call(self.erased_functions[&selection.entry], &values, "nominal.user.drop")
                .map_err(|error| Self::erased_error(error.to_string()))?;
            self.builder.build_return(None).map_err(|error| Self::erased_error(error.to_string()))?;
        }
        Ok(())
    }
}

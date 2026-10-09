use inkwell::module::Linkage;
use inkwell::types::StructType;
use inkwell::values::PointerValue;
use inkwell::{AddressSpace, IntPredicate};

use crate::codegen::{CodeGen, CodegenError};
use crate::ids::TypeId;

impl<'ctx> CodeGen<'ctx> {
    fn object_entry_type(&self) -> StructType<'ctx> {
        let ptr = self.context.ptr_type(AddressSpace::default());
        self.context.struct_type(
            &[ptr.into(), self.context.i64_type().into(), ptr.into()],
            false,
        )
    }

    pub(super) fn emit_object_entries(
        &self,
        object: TypeId,
        field: u32,
        methods: &[PointerValue<'ctx>],
        table: u32,
    ) -> Result<PointerValue<'ctx>, CodegenError> {
        let entry = self.object_entry_type();
        let values = methods
            .iter()
            .enumerate()
            .map(|(index, method)| {
                let (key, len) = self
                    .object_entry_keys
                    .get(&(object, field, index as u32))
                    .ok_or_else(|| {
                        CodegenError::backend_contract("object entry lacks member identity")
                    })?;
                Ok(entry.const_named_struct(&[
                    key.as_pointer_value().into(),
                    self.context.i64_type().const_int(*len, false).into(),
                    (*method).into(),
                ]))
            })
            .collect::<Result<Vec<_>, CodegenError>>()?;
        let value = entry.const_array(&values);
        let global = self.module.add_global(
            value.get_type(),
            None,
            &format!("rock.object.entries.{table}.{field}"),
        );
        global.set_linkage(Linkage::Private);
        global.set_constant(true);
        global.set_initializer(&value);
        Ok(global.as_pointer_value())
    }

    /// One allocation-free byte lookup shared by concrete and erased dispatch.
    /// No type encodings or borrowing semantics are interpreted at runtime.
    pub(super) fn emit_object_lookup(&self) -> Result<(), CodegenError> {
        if self.module.get_function("__rock_object_lookup").is_some() {
            return Ok(());
        }
        let word = self.context.i64_type();
        let ptr = self.context.ptr_type(AddressSpace::default());
        let function = self.module.add_function(
            "__rock_object_lookup",
            ptr.fn_type(&[ptr.into(), word.into(), ptr.into(), word.into()], false),
            Some(Linkage::Internal),
        );
        let trap = self.module.get_function("llvm.trap").unwrap_or_else(|| {
            self.module.add_function(
                "llvm.trap",
                self.context.void_type().fn_type(&[], false),
                None,
            )
        });
        let entry = self.context.append_basic_block(function, "entry");
        let scan = self.context.append_basic_block(function, "scan");
        let candidate = self.context.append_basic_block(function, "candidate");
        let compare = self.context.append_basic_block(function, "compare");
        let bytes = self.context.append_basic_block(function, "bytes");
        let byte = self.context.append_basic_block(function, "byte");
        let next_byte = self.context.append_basic_block(function, "next.byte");
        let found = self.context.append_basic_block(function, "found");
        let next = self.context.append_basic_block(function, "next");
        let missing = self.context.append_basic_block(function, "missing");
        let builder = self.context.create_builder();
        let args = function.get_param_iter().collect::<Vec<_>>();
        let array = args[0].into_pointer_value();
        let count = args[1].into_int_value();
        let key = args[2].into_pointer_value();
        let length = args[3].into_int_value();
        let build = || -> Result<(), inkwell::builder::BuilderError> {
            builder.position_at_end(entry);
            builder.build_unconditional_branch(scan)?;
            builder.position_at_end(scan);
            let index = builder.build_phi(word, "index")?;
            index.add_incoming(&[(&word.const_zero(), entry)]);
            let value = index.as_basic_value().into_int_value();
            let present = builder.build_int_compare(IntPredicate::ULT, value, count, "present")?;
            builder.build_conditional_branch(present, candidate, missing)?;
            builder.position_at_end(candidate);
            let address = unsafe {
                builder.build_gep(self.object_entry_type(), array, &[value], "candidate.entry")?
            };
            let size = builder.build_struct_gep(
                self.object_entry_type(),
                address,
                1,
                "candidate.length.address",
            )?;
            let size = builder
                .build_load(word, size, "candidate.length")?
                .into_int_value();
            let equal =
                builder.build_int_compare(IntPredicate::EQ, size, length, "equal.length")?;
            builder.build_conditional_branch(equal, compare, next)?;
            builder.position_at_end(compare);
            let actual = builder.build_struct_gep(
                self.object_entry_type(),
                address,
                0,
                "candidate.key.address",
            )?;
            let actual = builder
                .build_load(ptr, actual, "candidate.key")?
                .into_pointer_value();
            builder.build_unconditional_branch(bytes)?;
            builder.position_at_end(bytes);
            let offset = builder.build_phi(word, "byte.index")?;
            offset.add_incoming(&[(&word.const_zero(), compare)]);
            let offset_value = offset.as_basic_value().into_int_value();
            let present = builder.build_int_compare(
                IntPredicate::ULT,
                offset_value,
                length,
                "byte.present",
            )?;
            builder.build_conditional_branch(present, byte, found)?;
            builder.position_at_end(byte);
            let byte_type = self.context.i8_type();
            let (actual_byte, expected_byte) = unsafe {
                (
                    builder.build_gep(byte_type, actual, &[offset_value], "actual.byte.address")?,
                    builder.build_gep(byte_type, key, &[offset_value], "expected.byte.address")?,
                )
            };
            let actual_byte = builder
                .build_load(byte_type, actual_byte, "actual.byte")?
                .into_int_value();
            let expected_byte = builder
                .build_load(byte_type, expected_byte, "expected.byte")?
                .into_int_value();
            let equal = builder.build_int_compare(
                IntPredicate::EQ,
                actual_byte,
                expected_byte,
                "equal.byte",
            )?;
            builder.build_conditional_branch(equal, next_byte, next)?;
            builder.position_at_end(next_byte);
            let increment =
                builder.build_int_add(offset_value, word.const_int(1, false), "next.byte.index")?;
            offset.add_incoming(&[(&increment, next_byte)]);
            builder.build_unconditional_branch(bytes)?;
            builder.position_at_end(found);
            let code = builder.build_struct_gep(
                self.object_entry_type(),
                address,
                2,
                "candidate.code.address",
            )?;
            let code = builder.build_load(ptr, code, "candidate.code")?;
            builder.build_return(Some(&code))?;
            builder.position_at_end(next);
            let increment = builder.build_int_add(value, word.const_int(1, false), "next.index")?;
            index.add_incoming(&[(&increment, next)]);
            builder.build_unconditional_branch(scan)?;
            builder.position_at_end(missing);
            builder.build_call(trap, &[], "")?;
            builder.build_unreachable()?;
            Ok(())
        };
        build().map_err(|error| CodegenError::from(error.to_string()))
    }
}

use super::product_schema::SerializedProductArtifact;
use super::type_graph_validation::validate_type_table_graph;

pub(in crate::products) fn validate_portable_artifact_limits(
    artifact: &SerializedProductArtifact,
) -> Result<(), String> {
    validate_portable_artifact_limits_with_initial_string_bytes(artifact, 0)
}

pub(in crate::products) fn validate_portable_artifact_limits_with_initial_string_bytes(
    artifact: &SerializedProductArtifact,
    initial_string_bytes: usize,
) -> Result<(), String> {
    if artifact.format_version != crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION {
        return Err(format!(
            "Unsupported product artifact payload format {} (expected {})",
            artifact.format_version,
            crate::products::PRODUCT_ARTIFACT_FORMAT_VERSION
        ));
    }
    validate_serialized_product_strings(artifact, initial_string_bytes)?;
    if artifact.type_table.rows.len() > crate::products::MAX_PRODUCT_ARTIFACT_TYPE_ROWS {
        return Err(format!(
            "Product artifact type-table rows limit exceeded: declared {}, maximum {}",
            artifact.type_table.rows.len(),
            crate::products::MAX_PRODUCT_ARTIFACT_TYPE_ROWS
        ));
    }
    validate_type_table_graph(&artifact.type_table)?;

    let interface = &artifact.products.interface;
    let declaration_count = interface
        .functions
        .len()
        .saturating_add(interface.structs.len())
        .saturating_add(interface.enums.len())
        .saturating_add(interface.type_aliases.len())
        .saturating_add(interface.traits.len())
        .saturating_add(interface.impls.len())
        .saturating_add(interface.externs.len())
        .saturating_add(artifact.products.bodies.functions.len())
        .saturating_add(artifact.products.bodies.generic_impls.len())
        .saturating_add(artifact.products.bodies.trait_default_methods.len());
    if declaration_count > crate::products::MAX_PRODUCT_ARTIFACT_DECLARATIONS {
        return Err(format!(
            "Product artifact declarations limit exceeded: declared {declaration_count}, maximum {}",
            crate::products::MAX_PRODUCT_ARTIFACT_DECLARATIONS
        ));
    }

    let check_params = |owner: &str, count: usize| -> Result<(), String> {
        if count > crate::products::MAX_PRODUCT_ARTIFACT_GENERIC_PARAMS {
            return Err(format!(
                "Product artifact generic params limit exceeded for {owner}: declared {count}, maximum {}",
                crate::products::MAX_PRODUCT_ARTIFACT_GENERIC_PARAMS
            ));
        }
        Ok(())
    };
    for function in interface.functions.values() {
        check_params(&function.name, function.generic_params.len())?;
    }
    for structure in interface.structs.values() {
        check_params(&structure.name, structure.generic_params.len())?;
    }
    for enumeration in interface.enums.values() {
        check_params(&enumeration.name, enumeration.generic_params.len())?;
    }
    for alias in interface.type_aliases.values() {
        check_params(&alias.name, alias.generic_params.len())?;
    }
    for trait_def in interface.traits.values() {
        check_params(&trait_def.name, trait_def.generic_params.len())?;
        for method in trait_def.methods.values() {
            check_params(&method.name, method.generic_params.len())?;
        }
        for signature in trait_def.signatures.values() {
            check_params(&signature.name, signature.generic_params.len())?;
        }
    }
    for imp in interface.impls.values() {
        check_params(&imp.type_name, imp.type_generics.len())?;
        check_params(&imp.type_name, imp.trait_generics.len())?;
        for method in imp.methods.values() {
            check_params(&method.name, method.generic_params.len())?;
        }
    }
    for function in artifact
        .products
        .bodies
        .functions
        .values()
        .chain(artifact.products.bodies.trait_default_methods.values())
    {
        check_params(&function.name, function.generic_params.len())?;
    }
    for imp in artifact.products.bodies.generic_impls.values() {
        check_params(&imp.type_name, imp.type_generics.len())?;
        check_params(&imp.type_name, imp.trait_generics.len())?;
        for method in imp.methods.values() {
            check_params(&method.name, method.generic_params.len())?;
        }
    }
    Ok(())
}

#[derive(Debug)]
struct ProductStringLimitError(String);

impl std::fmt::Display for ProductStringLimitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ProductStringLimitError {}

impl serde::ser::Error for ProductStringLimitError {
    fn custom<T: std::fmt::Display>(message: T) -> Self {
        Self(message.to_string())
    }
}

struct ProductStringValidator {
    total: usize,
}

fn validate_serialized_product_strings(
    artifact: &SerializedProductArtifact,
    initial_string_bytes: usize,
) -> Result<(), String> {
    use serde::Serialize;

    let mut validator = ProductStringValidator {
        total: initial_string_bytes,
    };
    artifact
        .serialize(&mut validator)
        .map_err(|error| error.to_string())
}

impl ProductStringValidator {
    fn visit_str(&mut self, value: &str) -> Result<(), ProductStringLimitError> {
        crate::products::check_product_artifact_string(value, &mut self.total)
            .map_err(ProductStringLimitError)
    }
}

impl<'a> serde::Serializer for &'a mut ProductStringValidator {
    type Ok = ();
    type Error = ProductStringLimitError;
    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;

    fn serialize_bool(self, _: bool) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_i8(self, _: i8) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_i16(self, _: i16) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_i32(self, _: i32) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_i64(self, _: i64) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_i128(self, _: i128) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_u8(self, _: u8) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_u16(self, _: u16) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_u32(self, _: u32) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_u64(self, _: u64) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_u128(self, _: u128) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_f32(self, _: f32) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_f64(self, _: f64) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_char(self, _: char) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_str(self, value: &str) -> Result<Self::Ok, Self::Error> {
        self.visit_str(value)
    }

    fn serialize_bytes(self, _: &[u8]) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_none(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_some<T: ?Sized + serde::Serialize>(
        self,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
    ) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }

    fn serialize_newtype_struct<T: ?Sized + serde::Serialize>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + serde::Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        value.serialize(self)
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Self::Error> {
        Ok(self)
    }

    fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Self::Error> {
        Ok(self)
    }

    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleStruct, Self::Error> {
        Ok(self)
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeTupleVariant, Self::Error> {
        Ok(self)
    }

    fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Self::Error> {
        Ok(self)
    }

    fn serialize_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStruct, Self::Error> {
        Ok(self)
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> Result<Self::SerializeStructVariant, Self::Error> {
        Ok(self)
    }

    fn collect_str<T: ?Sized + std::fmt::Display>(
        self,
        value: &T,
    ) -> Result<Self::Ok, Self::Error> {
        self.visit_str(&value.to_string())
    }
}

impl serde::ser::SerializeSeq for &mut ProductStringValidator {
    type Ok = ();
    type Error = ProductStringLimitError;

    fn serialize_element<T: ?Sized + serde::Serialize>(
        &mut self,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl serde::ser::SerializeTuple for &mut ProductStringValidator {
    type Ok = ();
    type Error = ProductStringLimitError;

    fn serialize_element<T: ?Sized + serde::Serialize>(
        &mut self,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl serde::ser::SerializeTupleStruct for &mut ProductStringValidator {
    type Ok = ();
    type Error = ProductStringLimitError;

    fn serialize_field<T: ?Sized + serde::Serialize>(
        &mut self,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl serde::ser::SerializeTupleVariant for &mut ProductStringValidator {
    type Ok = ();
    type Error = ProductStringLimitError;

    fn serialize_field<T: ?Sized + serde::Serialize>(
        &mut self,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl serde::ser::SerializeMap for &mut ProductStringValidator {
    type Ok = ();
    type Error = ProductStringLimitError;

    fn serialize_key<T: ?Sized + serde::Serialize>(&mut self, key: &T) -> Result<(), Self::Error> {
        key.serialize(&mut **self)
    }

    fn serialize_value<T: ?Sized + serde::Serialize>(
        &mut self,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl serde::ser::SerializeStruct for &mut ProductStringValidator {
    type Ok = ();
    type Error = ProductStringLimitError;

    fn serialize_field<T: ?Sized + serde::Serialize>(
        &mut self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl serde::ser::SerializeStructVariant for &mut ProductStringValidator {
    type Ok = ();
    type Error = ProductStringLimitError;

    fn serialize_field<T: ?Sized + serde::Serialize>(
        &mut self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

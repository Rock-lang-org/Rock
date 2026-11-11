use std::collections::HashMap;

use super::type_rows::{
    ProductAssociatedTypeKey, ProductFunctionCapture, ProductGenericParamId, ProductTypeId,
    ProductTypeRow, ProductTypeTable,
};
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::products::{ProductCrateId, ProductDefId, ProductLocalDefId};
use crate::types::{AssociatedTypeKey, FunctionCapture, GenericParamId, Type};

#[derive(Debug, Default)]
pub(in crate::products) struct ProductTypeEncoder {
    table: ProductTypeTable,
    ids: HashMap<ProductTypeRow, ProductTypeId>,
}

impl ProductTypeEncoder {
    pub(in crate::products) fn new() -> Self {
        Self::default()
    }

    pub(in crate::products) fn finish(self) -> ProductTypeTable {
        self.table
    }

    pub(in crate::products) fn encode_type(&mut self, ty: &Type) -> Result<ProductTypeId, String> {
        let row = self.encode_row(ty)?;
        if let Some(id) = self.ids.get(&row).copied() {
            return Ok(id);
        }

        let id = ProductTypeId(self.table.rows.len() as u32);
        self.table.rows.push(row.clone());
        self.ids.insert(row, id);
        Ok(id)
    }

    pub(in crate::products) fn encode_types(
        &mut self,
        types: &[Type],
    ) -> Result<Vec<ProductTypeId>, String> {
        types.iter().map(|ty| self.encode_type(ty)).collect()
    }

    fn encode_row(&mut self, ty: &Type) -> Result<ProductTypeRow, String> {
        Ok(match ty {
            Type::I8 => ProductTypeRow::I8,
            Type::I16 => ProductTypeRow::I16,
            Type::I32 => ProductTypeRow::I32,
            Type::I64 => ProductTypeRow::I64,
            Type::U8 => ProductTypeRow::U8,
            Type::U16 => ProductTypeRow::U16,
            Type::U32 => ProductTypeRow::U32,
            Type::U64 => ProductTypeRow::U64,
            Type::F32 => ProductTypeRow::F32,
            Type::F64 => ProductTypeRow::F64,
            Type::Bool => ProductTypeRow::Bool,
            Type::Str => ProductTypeRow::Str,
            Type::Char => ProductTypeRow::Char,
            Type::Unit => ProductTypeRow::Unit,
            Type::Never => ProductTypeRow::Never,
            Type::ConstValue(n) => ProductTypeRow::ConstValue(*n),
            Type::Slice(inner) => ProductTypeRow::Slice(self.encode_type(inner)?),
            Type::Array(inner, len) => ProductTypeRow::Array(
                self.encode_type(inner)?,
                self.encode_type(&len.to_const_type())?,
            ),
            Type::Tuple(elems) => ProductTypeRow::Tuple(self.encode_types(elems)?),
            Type::Function {
                params,
                ret,
                safety,
                callable_kind,
                captures,
                captures_known,
            } => ProductTypeRow::Function {
                params: self.encode_types(params)?,
                ret: self.encode_type(ret)?,
                safety: (*safety).into(),
                callable_kind: (*callable_kind).into(),
                captures_known: *captures_known,
                captures: captures
                    .iter()
                    .map(|capture| {
                        Ok(ProductFunctionCapture {
                            kind: capture.kind.into(),
                            effect: capture.effect,
                            ty: self.encode_type(&capture.ty)?,
                        })
                    })
                    .collect::<Result<_, String>>()?,
            },
            Type::Struct { id, args } => ProductTypeRow::Struct {
                id: product_def_id(*id),
                args: self.encode_types(args)?,
            },
            Type::Enum { id, args } => ProductTypeRow::Enum {
                id: product_def_id(*id),
                args: self.encode_types(args)?,
            },
            Type::Reference { mutable, inner } => ProductTypeRow::Reference {
                mutable: *mutable,
                inner: self.encode_type(inner)?,
            },
            Type::Pointer(inner) => ProductTypeRow::Pointer(self.encode_type(inner)?),
            Type::TypeVar(_) => {
                return Err(
                    "product artifacts cannot serialize unresolved Type::TypeVar values"
                        .to_string(),
                );
            }
            Type::Generic(param) => ProductTypeRow::Generic(ProductGenericParamId {
                owner: product_def_id(param.owner),
                index: param.index,
            }),
            Type::Projection {
                ty,
                trait_id,
                assoc_type,
                trait_args,
            } => ProductTypeRow::Projection {
                ty: self.encode_type(ty)?,
                trait_id: product_def_id(*trait_id),
                assoc_type: ProductAssociatedTypeKey {
                    owner: product_def_id(assoc_type.owner),
                    assoc_type_id: assoc_type.assoc_type_id,
                },
                trait_args: self.encode_types(trait_args)?,
            },
            Type::Constructor { id, flavor } => ProductTypeRow::Constructor {
                id: product_def_id(*id),
                flavor: (*flavor).into(),
            },
            Type::Apply { constructor, args } => ProductTypeRow::Apply {
                constructor: self.encode_type(constructor)?,
                args: self.encode_types(args)?,
            },
            Type::Lambda { params, body } => ProductTypeRow::Lambda {
                params: params.clone(),
                body: self.encode_type(body)?,
            },
            Type::BoundVar { depth, index, kind } => ProductTypeRow::BoundVar {
                depth: *depth,
                index: *index,
                kind: kind.clone(),
            },
            Type::Error => {
                return Err(
                    "product artifacts cannot serialize unresolved Type::Error values".to_string(),
                );
            }
        })
    }
}

pub(in crate::products) struct ProductTypeDecoder<'a> {
    table: &'a ProductTypeTable,
    stack: Vec<ProductTypeId>,
}

impl<'a> ProductTypeDecoder<'a> {
    pub(in crate::products) fn new(table: &'a ProductTypeTable) -> Self {
        Self {
            table,
            stack: Vec::new(),
        }
    }

    pub(in crate::products) fn decode_type(&mut self, id: ProductTypeId) -> Result<Type, String> {
        let row = self
            .table
            .rows
            .get(id.0 as usize)
            .ok_or_else(|| format!("unknown product type id {}", id.0))?;
        if self.stack.contains(&id) {
            return Err(format!("recursive product type id {}", id.0));
        }

        self.stack.push(id);
        let decoded = self.decode_row(row);
        self.stack.pop();
        decoded
    }

    pub(in crate::products) fn decode_types(
        &mut self,
        ids: &[ProductTypeId],
    ) -> Result<Vec<Type>, String> {
        ids.iter().map(|id| self.decode_type(*id)).collect()
    }

    fn decode_row(&mut self, row: &ProductTypeRow) -> Result<Type, String> {
        Ok(match row {
            ProductTypeRow::I8 => Type::I8,
            ProductTypeRow::I16 => Type::I16,
            ProductTypeRow::I32 => Type::I32,
            ProductTypeRow::I64 => Type::I64,
            ProductTypeRow::U8 => Type::U8,
            ProductTypeRow::U16 => Type::U16,
            ProductTypeRow::U32 => Type::U32,
            ProductTypeRow::U64 => Type::U64,
            ProductTypeRow::F32 => Type::F32,
            ProductTypeRow::F64 => Type::F64,
            ProductTypeRow::Bool => Type::Bool,
            ProductTypeRow::Str => Type::Str,
            ProductTypeRow::Char => Type::Char,
            ProductTypeRow::Unit => Type::Unit,
            ProductTypeRow::Never => Type::Never,
            ProductTypeRow::ConstValue(n) => Type::ConstValue(*n),
            ProductTypeRow::Slice(inner) => Type::Slice(Box::new(self.decode_type(*inner)?)),
            ProductTypeRow::Array(inner, len) => {
                let inner = Box::new(self.decode_type(*inner)?);
                let len_ty = self.decode_type(*len)?;
                let len = crate::types::ArrayLen::try_from_const_type(&len_ty).ok_or_else(|| {
                    format!(
                        "array length {:?} is not a valid length",
                        len_ty
                    )
                })?;
                Type::Array(inner, len)
            }
            ProductTypeRow::Tuple(elems) => Type::Tuple(self.decode_types(elems)?),
            ProductTypeRow::Function {
                params,
                ret,
                safety,
                callable_kind,
                captures,
                captures_known,
            } => Type::Function {
                params: self.decode_types(params)?,
                ret: Box::new(self.decode_type(*ret)?),
                safety: (*safety).into(),
                callable_kind: (*callable_kind).into(),
                captures_known: *captures_known,
                captures: captures
                    .iter()
                    .map(|capture| {
                        Ok(FunctionCapture {
                            kind: capture.kind.into(),
                            effect: capture.effect,
                            ty: self.decode_type(capture.ty)?,
                        })
                    })
                    .collect::<Result<_, String>>()?,
            },
            ProductTypeRow::Struct { id, args } => Type::Struct {
                id: def_id(*id),
                args: self.decode_types(args)?,
            },
            ProductTypeRow::Enum { id, args } => Type::Enum {
                id: def_id(*id),
                args: self.decode_types(args)?,
            },
            ProductTypeRow::Reference { mutable, inner } => Type::Reference {
                mutable: *mutable,
                inner: Box::new(self.decode_type(*inner)?),
            },
            ProductTypeRow::Pointer(inner) => Type::Pointer(Box::new(self.decode_type(*inner)?)),
            ProductTypeRow::Generic(param) => Type::Generic(GenericParamId {
                owner: def_id(param.owner),
                index: param.index,
            }),
            ProductTypeRow::Projection {
                ty,
                trait_id,
                assoc_type,
                trait_args,
            } => Type::Projection {
                ty: Box::new(self.decode_type(*ty)?),
                trait_id: def_id(*trait_id),
                assoc_type: AssociatedTypeKey {
                    owner: def_id(assoc_type.owner),
                    assoc_type_id: assoc_type.assoc_type_id,
                },
                trait_args: self.decode_types(trait_args)?,
            },
            ProductTypeRow::Constructor { id, flavor } => Type::Constructor {
                id: def_id(*id),
                flavor: (*flavor).into(),
            },
            ProductTypeRow::Apply { constructor, args } => Type::Apply {
                constructor: Box::new(self.decode_type(*constructor)?),
                args: self.decode_types(args)?,
            },
            ProductTypeRow::Lambda { params, body } => Type::Lambda {
                params: params.clone(),
                body: Box::new(self.decode_type(*body)?),
            },
            ProductTypeRow::BoundVar { depth, index, kind } => Type::BoundVar {
                depth: *depth,
                index: *index,
                kind: kind.clone(),
            },
        })
    }
}

pub(super) fn product_def_id(id: DefId) -> ProductDefId {
    ProductDefId {
        crate_id: ProductCrateId(id.crate_id.0),
        local_id: ProductLocalDefId(id.local.0),
    }
}

pub(super) fn def_id(id: ProductDefId) -> DefId {
    DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0))
}

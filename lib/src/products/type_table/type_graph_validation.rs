use super::type_rows::{ProductTypeId, ProductTypeRow, ProductTypeTable};

pub(super) fn validate_type_table_graph(table: &ProductTypeTable) -> Result<(), String> {
    fn children(row: &ProductTypeRow) -> Vec<ProductTypeId> {
        match row {
            ProductTypeRow::Slice(inner)
            | ProductTypeRow::Pointer(inner)
            | ProductTypeRow::Reference { inner, .. } => vec![*inner],
            ProductTypeRow::Array(inner, len) => vec![*inner, *len],
            ProductTypeRow::Tuple(elements) => elements.clone(),
            ProductTypeRow::Function {
                params,
                ret,
                captures,
                ..
            } => params
                .iter()
                .copied()
                .chain(std::iter::once(*ret))
                .chain(captures.iter().map(|capture| capture.ty))
                .collect(),
            ProductTypeRow::Struct { args, .. } | ProductTypeRow::Enum { args, .. } => args.clone(),
            ProductTypeRow::Projection { ty, trait_args, .. } => std::iter::once(*ty)
                .chain(trait_args.iter().copied())
                .collect(),
            ProductTypeRow::Apply { constructor, args } => std::iter::once(*constructor)
                .chain(args.iter().copied())
                .collect(),
            ProductTypeRow::Lambda { body, .. } => vec![*body],
            ProductTypeRow::I8
            | ProductTypeRow::I16
            | ProductTypeRow::I32
            | ProductTypeRow::I64
            | ProductTypeRow::U8
            | ProductTypeRow::U16
            | ProductTypeRow::U32
            | ProductTypeRow::U64
            | ProductTypeRow::F32
            | ProductTypeRow::F64
            | ProductTypeRow::Bool
            | ProductTypeRow::Str
            | ProductTypeRow::Char
            | ProductTypeRow::Unit
            | ProductTypeRow::Never
            | ProductTypeRow::ConstValue(_)
            | ProductTypeRow::Generic(_)
            | ProductTypeRow::Constructor { .. }
            | ProductTypeRow::BoundVar { .. } => Vec::new(),
        }
    }

    fn visit(
        table: &ProductTypeTable,
        id: ProductTypeId,
        binders: &mut Vec<usize>,
        stack: &mut Vec<ProductTypeId>,
        nodes: &mut usize,
    ) -> Result<(), String> {
        if stack.len() >= crate::products::MAX_PRODUCT_ARTIFACT_TYPE_DEPTH {
            return Err(format!(
                "Product artifact type depth limit exceeded: maximum {}",
                crate::products::MAX_PRODUCT_ARTIFACT_TYPE_DEPTH
            ));
        }
        if stack.contains(&id) {
            return Err(format!("recursive product type id {}", id.0));
        }
        let row = table
            .rows
            .get(id.0 as usize)
            .ok_or_else(|| format!("unknown product type id {}", id.0))?;
        *nodes = nodes.saturating_add(1);
        if *nodes > crate::products::MAX_PRODUCT_ARTIFACT_NORMALIZATION_NODES {
            return Err(format!(
                "Product artifact normalization output nodes limit exceeded: maximum {}",
                crate::products::MAX_PRODUCT_ARTIFACT_NORMALIZATION_NODES
            ));
        }
        if let ProductTypeRow::BoundVar { depth, index, .. } = row {
            let depth = *depth as usize;
            let Some(scope) = binders.iter().rev().nth(depth) else {
                return Err(format!("out-of-scope product bound variable depth {depth}"));
            };
            if *index as usize >= *scope {
                return Err(format!(
                    "out-of-scope product bound variable index {index} for binder width {scope}"
                ));
            }
            return Ok(());
        }
        stack.push(id);
        if let ProductTypeRow::Lambda { params, body } = row {
            if params.len() > crate::products::MAX_PRODUCT_ARTIFACT_GENERIC_PARAMS {
                return Err(format!(
                    "Product artifact lambda params limit exceeded: declared {}, maximum {}",
                    params.len(),
                    crate::products::MAX_PRODUCT_ARTIFACT_GENERIC_PARAMS
                ));
            }
            binders.push(params.len());
            visit(table, *body, binders, stack, nodes)?;
            binders.pop();
        } else {
            for child in children(row) {
                visit(table, child, binders, stack, nodes)?;
            }
        }
        stack.pop();
        Ok(())
    }

    let mut referenced = vec![false; table.rows.len()];
    for row in &table.rows {
        for child in children(row) {
            let Some(slot) = referenced.get_mut(child.0 as usize) else {
                return Err(format!("unknown product type id {}", child.0));
            };
            *slot = true;
        }
    }
    let mut nodes = 0usize;
    for (index, is_referenced) in referenced.iter().enumerate() {
        if !is_referenced {
            visit(
                table,
                ProductTypeId(index as u32),
                &mut Vec::new(),
                &mut Vec::new(),
                &mut nodes,
            )?;
        }
    }
    let mut state = vec![0; table.rows.len()];
    for index in 0..table.rows.len() {
        if state[index] != 0 {
            continue;
        }
        let mut pending = vec![(ProductTypeId(index as u32), false)];
        while let Some((id, exiting)) = pending.pop() {
            let index = id.0 as usize;
            if exiting {
                state[index] = 2;
                continue;
            }
            match state[index] {
                1 => return Err(format!("recursive product type id {}", id.0)),
                2 => continue,
                _ => {}
            }
            state[index] = 1;
            pending.push((id, true));
            for child in children(&table.rows[index]).into_iter().rev() {
                if state[child.0 as usize] == 1 {
                    return Err(format!("recursive product type id {}", child.0));
                }
                if state[child.0 as usize] == 0 {
                    pending.push((child, false));
                }
            }
        }
    }
    Ok(())
}

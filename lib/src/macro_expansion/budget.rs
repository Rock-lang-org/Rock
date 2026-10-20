use std::collections::HashMap;

use crate::ast::InvocationId;

/// Compiler-controlled aggregate limits, shared by all rounds and siblings.
#[derive(Debug, Clone)]
pub struct ExpansionLimits {
    pub max_invocations: usize,
    pub max_output_bytes: usize,
    pub max_output_units: usize,
    pub max_matching_work: usize,
    pub max_source_discovery_work: usize,
    pub max_source_bytes: usize,
}

impl Default for ExpansionLimits {
    fn default() -> Self {
        Self {
            max_invocations: 10_000,
            max_output_bytes: 64 * 1024 * 1024,
            max_output_units: 4 * 1024 * 1024,
            max_matching_work: 32 * 1024 * 1024,
            max_source_discovery_work: 100_000,
            max_source_bytes: 64 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct Budget {
    pub limits: ExpansionLimits,
    pub executed: HashMap<InvocationId, super::ExpansionId>,
    pub source_parents: HashMap<Vec<String>, super::ExpansionId>,
    pub output_bytes: usize,
    pub output_units: usize,
    pub matching_work: usize,
    pub source_work: usize,
    pub source_bytes: usize,
}

pub(super) fn charge(used: &mut usize, amount: usize, limit: usize, label: &str) -> Result<(), String> {
    *used = used.checked_add(amount).filter(|total| *total <= limit)
        .ok_or_else(|| format!("macro {label} budget exceeded"))?;
    Ok(())
}

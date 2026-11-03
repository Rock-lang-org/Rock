use std::fs;
use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::crate_system::CrateContext;
use crate::hir::{
    AcceptedHirExpr, AcceptedHirFunction, HirExprKindFor as HirExprKind, HirSelectedMethodTarget,
    HirStmtFor as HirStmt,
};
use crate::ids::{CrateId, DefId, LocalDefId};
use crate::products::{CompilerProducts, ProductDefId, ProductLanguageItems};
use crate::types::Type;
use crate::Config;

#[path = "../../../tests/support/std_cache_key.rs"]
mod std_cache_key;
use std_cache_key::{std_cache_compiler_stamp, std_cache_key};

mod generic_bodies;
mod harness;
mod language_items;
mod link_symbols;
mod providers;
mod source_free;
mod std_artifact;
mod support;
mod trait_defaults;

use support::*;

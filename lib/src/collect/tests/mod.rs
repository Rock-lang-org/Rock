use super::*;

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::ast::{
    FunctionDecl, FunctionSig, Ident, Impl, LambdaArrowKind, LambdaDecl, Module, ModuleDecl,
    ParseType, ParseTypeInner, Pattern, PatternKind, Program, StructDecl, TopLevel,
};
use crate::crate_artifact::{ArtifactCrateInterface, ArtifactExport};
use crate::hir::{
    HirBlock, HirEnum, HirFunction, HirFunctionSig, HirMethodLocation, HirStruct, HirTrait,
};
use crate::ids::{AssocTypeId, CrateId, DefId, LocalDefId, ModuleId, VariantId};
use crate::language_items::{
    DropLanguageItems, LanguageItems, SizedLanguageItems, TryLanguageItems,
};
use crate::lexer::Span;
use crate::products::{
    CompilerProducts, ProductCrateIdentity, ProductDefId, ProductLinkData, ProductSourceFingerprint,
};
use crate::type_services::kind::Kind;
use crate::types::{GenericParamDecl, GenericParamId, Type};

mod support;
use support::*;

mod artifact_declarations;
mod declarations;
mod dependencies;
mod ids;
mod imports;
mod language_items;
mod modules;

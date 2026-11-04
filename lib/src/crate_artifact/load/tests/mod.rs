use super::body_remap::remap_function_child_locations;
use super::child_location_validator::{
    ProductChildLocationValidator, MAX_ARTIFACT_HIR_REMAP_DEPTH,
};
use super::dependency_definitions::ProductDependencyDefinitions;
use super::extern_crate_record::{artifact_hir_name_tables, intern_loaded_interface_types};
use super::generic_remap::remap_type_def_ids;
use super::method_authority::validate_product_method_authority_maps;
use super::nominal_validator::ProductNominalTypeValidator;
use super::product_validation::{validate_product_interface_rows, validate_product_type_fields};
use super::products_to_interface::{cross_crate_hir_from_products, resolver_from_products};
use super::target_remap::{remap_pattern_child_locations, remap_var_target_child_location};
use super::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::PathBuf;

use crate::collect::resolver::ResolverTables;
use crate::crate_artifact::{ArtifactCrateInterface, ArtifactCrossCrateHir};
use crate::crate_system::{
    CrateContext, ExternCrateBodies, ExternCrateLink, ExternCrateMetadata, ExternCrateRecord,
};
use crate::hir::{
    AcceptedHir, AcceptedHirBlock as HirBlock, AcceptedHirExpr as HirExpr,
    AcceptedHirFunction as HirFunction, AcceptedHirImpl as HirImpl, AcceptedHirTrait as HirTrait,
    HirAssociatedTypeDecl, HirCallTarget, HirClosureCapture, HirClosureCaptureKind, HirEnum,
    HirExprKindFor, HirExtern, HirField, HirFieldLocation, HirFunctionSig, HirImplOwner,
    HirMethodCallTarget, HirParam, HirPattern, HirStruct, HirStructPatternField, HirVarRef,
    HirVarTarget, HirVariant, HirVariantFields, HirVariantLocation,
};
use crate::ids::{AssocTypeId, CrateId, DefId, FieldId, LocalDefId, VariantId};
use crate::language_items::DropLanguageItems;
use crate::lexer::Span;
use crate::products::{
    CompilerProducts, ProductBodies, ProductCrateId, ProductCrateIdentity, ProductDefId,
    ProductEnumInterface, ProductExternInterface, ProductFieldInterface, ProductFunctionInterface,
    ProductIdentityTable, ProductImplInterface, ProductInterface, ProductLinkData,
    ProductLinkRecord, ProductLocalDefId, ProductSourceFingerprint, ProductStructInterface,
    ProductTraitInterface, PRODUCT_ARTIFACT_MAGIC,
};
use crate::type_services::kind::Kind;
use crate::types::{AssociatedTypeKey, GenericParamDecl, GenericParamId, Type};

mod aliases;
mod call_targets;
mod dependency_identity;
mod generics;
mod interface_bodies;
mod language_items;
mod link_records;
mod method_targets;
mod nominal_types;
mod prelude_root_exports;
mod remap_locations;
mod sidecars;
mod trait_targets;
mod transactional;

type HirStmt = crate::hir::HirStmtFor<AcceptedHir>;

fn product_def_id(crate_id: u32, local_id: u32) -> ProductDefId {
    ProductDefId {
        crate_id: ProductCrateId(crate_id),
        local_id: ProductLocalDefId(local_id),
    }
}

fn product_hir_def_id(id: ProductDefId) -> DefId {
    DefId::new(CrateId(id.crate_id.0), LocalDefId(id.local_id.0))
}

fn generic_params(owner: DefId, names: &[&str]) -> Vec<GenericParamDecl> {
    GenericParamDecl::type_params(owner, names.iter().copied())
}

fn test_function(id: DefId, name: &str) -> HirFunction {
    HirFunction {
        id,
        name: name.to_string(),
        generic_params: Vec::new(),
        generic_bounds: HashMap::new().into(),
        params: Vec::new(),
        ret_type: Type::I64,
        body: HirBlock {
            stmts: vec![HirStmt::Expr(HirExpr {
                kind: HirExprKindFor::IntLiteral(5),
                ty: Type::I64,
                span: Span::test(),
            })],
            ty: Type::I64,
        },
        is_curried: false,
        is_method: false,
        self_receiver: None,
        is_unsafe: false,
    }
}

fn test_generic_impl(id: DefId, methods: HashMap<String, HirFunction>) -> HirImpl {
    let receiver_pattern = methods
        .values()
        .find(|method| method.is_method)
        .and_then(|method| method.params.first())
        .map(|param| match &param.ty {
            Type::Reference { inner, .. } => inner.as_ref().clone(),
            ty => ty.clone(),
        })
        .map(crate::hir::HirImplReceiverPattern::Exact)
        .unwrap_or_else(|| {
            crate::hir::HirImplReceiverPattern::Exact(Type::Generic(crate::types::GenericParamId {
                owner: id,
                index: 0,
            }))
        });
    HirImpl {
        id,
        owner: HirImplOwner::Named("Box".to_string()),
        type_name: "Box".to_string(),
        type_generics: generic_params(id, &["T"]),
        receiver_pattern,
        trait_name: None,
        trait_id: None,
        trait_generics: Vec::new(),
        trait_arg_types: Vec::new(),
        associated_types: Vec::new(),
        bounds: HashMap::new().into(),
        methods,
    }
}

fn insert_interface_function(
    products: &mut CompilerProducts,
    id: ProductDefId,
    function: &HirFunction,
) {
    products
        .interface
        .functions
        .insert(id, ProductFunctionInterface::from(function));
}

fn insert_function_body(products: &mut CompilerProducts, id: ProductDefId, function: HirFunction) {
    insert_interface_function(products, id, &function);
    products.bodies.functions.insert(id, function);
}

fn insert_interface_struct(products: &mut CompilerProducts, id: ProductDefId, strukt: HirStruct) {
    products
        .interface
        .structs
        .insert(id, ProductStructInterface::from(&strukt));
}

fn insert_interface_enum(products: &mut CompilerProducts, id: ProductDefId, enm: HirEnum) {
    products
        .interface
        .enums
        .insert(id, ProductEnumInterface::from(&enm));
}

fn insert_interface_trait(products: &mut CompilerProducts, id: ProductDefId, trait_def: HirTrait) {
    products
        .interface
        .traits
        .insert(id, ProductTraitInterface::from(&trait_def));
}

fn insert_interface_impl(products: &mut CompilerProducts, id: ProductDefId, imp: HirImpl) {
    products
        .interface
        .impls
        .insert(id, ProductImplInterface::from(&imp));
}

fn insert_interface_extern(products: &mut CompilerProducts, id: ProductDefId, ext: HirExtern) {
    products
        .interface
        .externs
        .insert(id, ProductExternInterface::from(&ext));
}

fn product_with_function(
    crate_name: &str,
    product_crate_id: ProductCrateId,
    local_id: u32,
    object_path: PathBuf,
) -> CompilerProducts {
    let product_id = ProductDefId {
        crate_id: product_crate_id,
        local_id: ProductLocalDefId(local_id),
    };
    let function_id = DefId::new(CrateId(product_crate_id.0), LocalDefId(local_id));
    let function_name = format!("{}::answer", crate_name);
    let function = test_function(function_id, &function_name);

    let mut identity_table = ProductIdentityTable::default();
    identity_table.local_crate = Some(product_crate_id);
    identity_table
        .display_names
        .insert(product_id, function_name.clone());
    identity_table
        .export_names
        .insert("answer".to_string(), product_id);

    let mut bodies = ProductBodies::default();
    bodies.functions.insert(product_id, function);

    let mut interface = ProductInterface::default();
    interface.functions.insert(
        product_id,
        ProductFunctionInterface::from(&bodies.functions[&product_id]),
    );

    let mut link_records = BTreeMap::new();
    link_records.insert(
        product_id,
        ProductLinkRecord {
            backend_symbol: format!("{}_answer", crate_name),
        },
    );

    CompilerProducts {
        crate_identity: ProductCrateIdentity::local(crate_name.to_string()),
        identity_table,
        interface,
        bodies,
        link: ProductLinkData {
            object_path: Some(object_path),
            records: link_records,
        },
        dependencies: Vec::new(),
        source_fingerprint: ProductSourceFingerprint::default(),
        infix_precedence: Default::default(),
        proc_macros: Vec::new(),
    }
}

fn write_product_fixture(base: &std::path::Path, crate_name: &str) -> PathBuf {
    let artifact_path = base.join(format!("{}.rkca", crate_name));
    let object_path = base.join(format!("{}.o", crate_name));
    fs::write(&object_path, []).unwrap();

    let products = product_with_function(crate_name, ProductCrateId(0), 0, object_path);
    products.write_artifact_to_path(&artifact_path).unwrap();

    artifact_path
}

struct TempDirCleanup(PathBuf);

impl Drop for TempDirCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn temp_test_dir(label: &str) -> (PathBuf, TempDirCleanup) {
    let base = std::env::temp_dir().join(format!(
        "rock_product_loader_{}_{}",
        std::process::id(),
        label
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).unwrap();
    let cleanup = TempDirCleanup(base.clone());
    (base, cleanup)
}

fn validate_artifact_expr(products: &CompilerProducts, expr: &mut HirExpr) -> Result<(), String> {
    let remap = super::ProductIdentityRemap {
        local_crate: ProductCrateId(0),
        crate_ids: BTreeMap::from([(ProductCrateId(0), CrateId(7))]),
    };
    let child_validator = ProductChildLocationValidator::from_products(products);
    let type_validator = ProductNominalTypeValidator::from_products(products, &remap);
    remap_expr_child_locations(expr, &remap, &child_validator, &type_validator, 0)
}

impl CrateContext {
    pub(crate) fn load_product_artifact_from_path_as(
        &mut self,
        expected_name: &str,
        artifact_path: PathBuf,
    ) -> Result<(), String> {
        let mut context = crate::type_context::TypeContext::new();
        self.load_product_artifact_from_path_as_with_type_context(
            expected_name,
            artifact_path,
            &mut context,
        )
    }

    pub(crate) fn load_product_artifact_from_path_as_with_type_context(
        &mut self,
        expected_name: &str,
        artifact_path: PathBuf,
        context: &mut crate::type_context::TypeContext,
    ) -> Result<(), String> {
        let header = ProductArtifactHeader::read_from_path_bounded(&artifact_path)?;
        self.load_product_artifact_from_header_as_with_type_context(
            expected_name,
            artifact_path,
            &header,
            context,
        )
    }
}

pub(crate) fn remap_expr_child_locations<P: crate::hir::HirPhase>(
    expr: &mut crate::hir::HirExprFor<P>,
    remap: &ProductIdentityRemap,
    validator: &ProductChildLocationValidator,
    type_validator: &ProductNominalTypeValidator,
    depth: usize,
) -> Result<(), String> {
    let mut local_scope = BTreeSet::new();
    super::body_remap::remap_expr_child_locations_with_scope(
        expr,
        remap,
        validator,
        type_validator,
        &mut local_scope,
        depth,
    )
}

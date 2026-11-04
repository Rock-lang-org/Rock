use super::*;

impl ProductNominalTypeValidator {
    pub(crate) fn from_products(products: &CompilerProducts, remap: &ProductIdentityRemap) -> Self {
        Self::from_products_with_dependencies(
            products,
            remap,
            ProductDependencyDefinitions::default(),
        )
    }
}

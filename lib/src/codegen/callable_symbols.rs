use std::collections::HashMap;

use crate::ids::DefId;
use crate::mir::{MirCallableDecl, MirCallableKey, MirCallableKind, MirCallableSignature, MirFunctionId};

/// Symbol and signature bookkeeping for MIR callables: which LLVM symbol each callable key
/// and MIR function was declared under, and the ABI signature behind each symbol.
#[derive(Default)]
pub(super) struct CallableSymbols {
    function_symbols: HashMap<MirFunctionId, String>,
    symbols_by_key: HashMap<MirCallableKey, String>,
    signatures_by_key: HashMap<MirCallableKey, MirCallableSignature>,
    signatures_by_symbol: HashMap<String, MirCallableSignature>,
    contract_overrides: HashMap<DefId, String>,
    /// Namespace prefixed onto exported Rock function symbols of this crate.
    namespace: Option<String>,
}

impl CallableSymbols {
    pub(super) fn with_namespace(namespace: Option<String>) -> Self {
        Self {
            namespace,
            ..Self::default()
        }
    }

    pub(super) fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    /// Records the contract-declared symbol and signature of a callable before it is declared
    /// in LLVM.
    pub(super) fn register_declaration(&mut self, declaration: &MirCallableDecl) {
        self.symbols_by_key
            .insert(declaration.key.clone(), declaration.llvm_symbol.clone());
        self.signatures_by_key
            .insert(declaration.key.clone(), declaration.signature.clone());
        self.signatures_by_symbol.insert(
            declaration.llvm_symbol.clone(),
            declaration.signature.clone(),
        );
        self.bind_body_symbol(declaration, &declaration.llvm_symbol);
    }

    /// Re-points a declaration at the symbol it was actually emitted under, when that differs
    /// from the contract symbol.
    pub(super) fn register_actual_symbol(&mut self, declaration: &MirCallableDecl, symbol: &str) {
        self.bind_key_symbol(declaration.key.clone(), symbol);
        if let Some(def_id) = declaration.source_def_id {
            self.contract_overrides.insert(def_id, symbol.to_string());
        }
        self.bind_body_symbol(declaration, symbol);
    }

    fn bind_body_symbol(&mut self, declaration: &MirCallableDecl, symbol: &str) {
        if let MirCallableKey::Closure(function_id) = &declaration.key {
            self.function_symbols
                .insert(function_id.clone(), symbol.to_string());
        }

        if let MirCallableKind::LocalBody { function_id } = &declaration.kind {
            self.function_symbols
                .insert(function_id.clone(), symbol.to_string());
        }
    }

    pub(super) fn bind_key_symbol(&mut self, key: MirCallableKey, symbol: &str) {
        self.symbols_by_key.insert(key, symbol.to_string());
    }

    pub(super) fn bind_function_symbol(&mut self, id: MirFunctionId, symbol: String) {
        self.function_symbols.insert(id, symbol);
    }

    pub(super) fn bind_symbol_signature(&mut self, symbol: &str, signature: &MirCallableSignature) {
        self.signatures_by_symbol
            .insert(symbol.to_string(), signature.clone());
    }

    pub(super) fn symbol_for_key(&self, key: &MirCallableKey) -> Option<&String> {
        self.symbols_by_key.get(key)
    }

    pub(super) fn signature_for_key(&self, key: &MirCallableKey) -> Option<&MirCallableSignature> {
        self.signatures_by_key.get(key)
    }

    pub(super) fn function_symbol(&self, id: &MirFunctionId) -> Option<&String> {
        self.function_symbols.get(id)
    }

    pub(super) fn signature_for_function(&self, id: &MirFunctionId) -> Option<&MirCallableSignature> {
        self.function_symbol(id)
            .and_then(|symbol| self.signatures_by_symbol.get(symbol))
    }

    pub(super) fn contract_symbol_overrides(&self) -> &HashMap<DefId, String> {
        &self.contract_overrides
    }
}

use crate::ids::{DefId, TypeId};
use crate::products::ProductCrateIdentity;
use crate::type_context::Ty;
use crate::types::Type;

use super::{InstanceImplOwner, InstanceOrigin, Monomorphizer};

impl Monomorphizer {
    /// Instance identities exist during recursive specialization, but backend
    /// symbols must not be materialized until every instantiated body is valid.
    pub(super) fn finish_instance_symbols(&mut self) {
        self.check_instance_array_lengths();
        if self.diagnostics.return_if_err().is_err() {
            return;
        }
        let pending = self.instances.records()
            .filter(|record| !record.provided_by_object && record.symbols.backend_symbol.is_empty())
            .map(|record| (record.id, record.origin.clone(), record.substitution.clone(), record.symbols.source_name.clone()))
            .collect::<Vec<_>>();
        for (id, origin, substitution, source_name) in pending {
            let symbol = match origin {
                InstanceOrigin::Function(_) => self.backend_symbol_for_function(&source_name, &origin, &substitution),
                _ => self.backend_symbol_for_origin(&origin, &substitution),
            };
            self.instances.set_backend_symbol(id, symbol);
        }
    }

    fn hash_canonical(bytes: &[u8]) -> u64 {
        let mut hash = 0xcbf29ce484222325u64;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash
    }

    fn push_text(output: &mut String, text: &str) {
        output.push_str(&text.len().to_string());
        output.push(':');
        output.push_str(text);
    }

    fn write_crate_identity(output: &mut String, identity: &ProductCrateIdentity) {
        Self::push_text(output, &identity.name);
        Self::push_text(output, &identity.version);
        Self::push_text(output, identity.target_triple.as_deref().unwrap_or(""));
        Self::push_text(output, &identity.format_version.to_string());
        Self::push_text(
            output,
            identity
                .source_fingerprint
                .manifest_hash
                .as_deref()
                .unwrap_or(""),
        );
        Self::push_text(
            output,
            identity
                .source_fingerprint
                .source_hash
                .as_deref()
                .unwrap_or(""),
        );
        Self::push_text(
            output,
            &identity.source_fingerprint.loaded_files.len().to_string(),
        );
        for path in &identity.source_fingerprint.loaded_files {
            Self::push_text(output, &path.to_string_lossy());
        }
    }

    fn write_def_id(&self, output: &mut String, def_id: DefId) {
        output.push('D');
        Self::write_crate_identity(
            output,
            self.identity_context.crate_identity(def_id.crate_id),
        );
        Self::push_text(output, &def_id.local.0.to_string());
    }

    fn write_kind(output: &mut String, kind: &crate::type_services::kind::Kind) {
        match kind {
            crate::type_services::kind::Kind::Type => output.push('T'),
            crate::type_services::kind::Kind::Usize => output.push('U'),
            crate::type_services::kind::Kind::Arrow(input, output_kind) => {
                output.push('K');
                Self::write_kind(output, input);
                Self::write_kind(output, output_kind);
            }
        }
    }

    fn write_type_id(&self, output: &mut String, id: TypeId) {
        Self::write_kind(output, self.type_context.kind(id));
        match self.type_context.ty(id) {
            Ty::I8 => output.push_str("i8"),
            Ty::I16 => output.push_str("i16"),
            Ty::I32 => output.push_str("i32"),
            Ty::I64 => output.push_str("i64"),
            Ty::U8 => output.push_str("u8"),
            Ty::U16 => output.push_str("u16"),
            Ty::U32 => output.push_str("u32"),
            Ty::U64 => output.push_str("u64"),
            Ty::F32 => output.push_str("f32"),
            Ty::F64 => output.push_str("f64"),
            Ty::Bool => output.push('b'),
            Ty::Str => output.push_str("str"),
            Ty::Char => output.push('c'),
            Ty::Unit => output.push('u'),
            Ty::Never => output.push('n'),
            Ty::ConstValue(n) => {
                // Distinct tag from `Char` and length-delimited like other numeric tokens.
                output.push('v');
                Self::push_text(output, &n.to_string());
            }
            Ty::Slice(inner) => {
                output.push('s');
                self.write_type_id(output, *inner);
            }
            Ty::Array { inner, len } => {
                output.push('a');
                match len.concrete() {
                    Some(len) => Self::push_text(output, &len.to_string()),
                    // Recovery must not merge distinct symbolic substitutions.
                    None => Self::push_text(output, &format!("{len:?}")),
                }
                self.write_type_id(output, *inner);
            }
            Ty::Tuple(elements) => {
                output.push('t');
                Self::push_text(output, &elements.len().to_string());
                for element in elements {
                    self.write_type_id(output, *element);
                }
            }
            Ty::Function {
                params,
                ret,
                safety,
                callable_kind,
                captures,
            } => {
                output.push('f');
                Self::push_text(output, &format!("{safety:?}"));
                Self::push_text(output, &format!("{callable_kind:?}"));
                Self::push_text(output, &params.len().to_string());
                for param in params {
                    self.write_type_id(output, *param);
                }
                self.write_type_id(output, *ret);
                Self::push_text(output, &captures.len().to_string());
                for capture in captures {
                    Self::push_text(output, &format!("{:?}", capture.kind));
                    Self::push_text(output, &format!("{:?}", capture.effect));
                    self.write_type_id(output, capture.ty);
                }
            }
            Ty::Struct { id, args } => {
                output.push('S');
                self.write_def_id(output, *id);
                Self::push_text(output, &args.len().to_string());
                for arg in args {
                    self.write_type_id(output, *arg);
                }
            }
            Ty::Enum { id, args } => {
                output.push('E');
                self.write_def_id(output, *id);
                Self::push_text(output, &args.len().to_string());
                for arg in args {
                    self.write_type_id(output, *arg);
                }
            }
            Ty::Reference { mutable, inner } => {
                output.push(if *mutable { 'R' } else { 'r' });
                self.write_type_id(output, *inner);
            }
            Ty::Pointer(inner) => {
                output.push('p');
                self.write_type_id(output, *inner);
            }
            Ty::Generic(param) => {
                output.push('g');
                self.write_def_id(output, param.owner);
                Self::push_text(output, &param.index.to_string());
            }
            Ty::Projection {
                ty,
                trait_id,
                assoc_type,
                trait_args,
            } => {
                output.push('q');
                self.write_type_id(output, *ty);
                self.write_def_id(output, *trait_id);
                self.write_def_id(output, assoc_type.owner);
                Self::push_text(output, &assoc_type.assoc_type_id.0.to_string());
                Self::push_text(output, &trait_args.len().to_string());
                for arg in trait_args {
                    self.write_type_id(output, *arg);
                }
            }
            Ty::Constructor { id, flavor } => {
                output.push('C');
                Self::push_text(output, &format!("{flavor:?}"));
                self.write_def_id(output, *id);
            }
            Ty::Apply { constructor, args } => {
                output.push('A');
                self.write_type_id(output, *constructor);
                Self::push_text(output, &args.len().to_string());
                for arg in args {
                    self.write_type_id(output, *arg);
                }
            }
            Ty::Lambda { params, body } => {
                output.push('L');
                Self::push_text(output, &params.len().to_string());
                for param in params {
                    Self::write_kind(output, param);
                }
                self.write_type_id(output, *body);
            }
            Ty::BoundVar { depth, index, kind } => {
                output.push('B');
                Self::push_text(output, &depth.to_string());
                Self::push_text(output, &index.to_string());
                Self::write_kind(output, kind);
            }
            Ty::TypeVar(_) | Ty::Error => {
                panic!("unstable type state reached backend symbol fingerprint")
            }
        }
    }

    fn def_symbol_fragment(&self, def_id: DefId) -> String {
        let mut canonical = String::new();
        self.write_def_id(&mut canonical, def_id);
        format!(
            "d{}_h{:016x}",
            def_id.local.0,
            Self::hash_canonical(canonical.as_bytes())
        )
    }

    fn substitution_symbol_suffix(&self, substitution: &[TypeId]) -> String {
        if substitution.is_empty() {
            return "none".to_string();
        }

        let mut canonical = String::new();
        Self::push_text(&mut canonical, &substitution.len().to_string());
        for ty in substitution {
            self.write_type_id(&mut canonical, *ty);
        }
        format!("h{:016x}", Self::hash_canonical(canonical.as_bytes()))
    }

    pub(super) fn backend_symbol_for_origin(
        &mut self,
        origin: &InstanceOrigin,
        substitution: &[TypeId],
    ) -> String {
        if substitution.iter().any(|id| {
            super::array_lengths::unresolved_array_length(&self.type_context.type_for(*id))
        }) {
            self.diagnostics.push(
                crate::diagnostic::Diagnostic::for_internal(
                    "cannot generate an instance symbol with an unresolved array length",
                )
                .with_code(crate::diagnostic::DiagnosticCode::Mono),
            );
        }
        let suffix = self.substitution_symbol_suffix(substitution);
        match origin {
            InstanceOrigin::Function(def_id) => {
                format!("__rock_fn_{}_{}", self.def_symbol_fragment(*def_id), suffix)
            }
            InstanceOrigin::ImplMethod { owner, method } => {
                let owner = match owner {
                    InstanceImplOwner::Named(owner) => self.def_symbol_fragment(*owner),
                    InstanceImplOwner::BuiltinSlice => "builtin_slice".to_string(),
                };
                format!(
                    "__rock_impl_{}_method_{}_{}",
                    owner,
                    self.def_symbol_fragment(*method),
                    suffix
                )
            }
            InstanceOrigin::TraitDefault { trait_id, method } => {
                format!(
                    "__rock_trait_default_{}_method_{}_{}",
                    self.def_symbol_fragment(*trait_id),
                    self.def_symbol_fragment(*method),
                    suffix
                )
            }
        }
    }

    pub(super) fn backend_symbol_for_function(
        &mut self,
        name: &str,
        origin: &InstanceOrigin,
        substitution: &[TypeId],
    ) -> String {
        if name == "main" && substitution.is_empty() {
            "main".to_string()
        } else {
            self.backend_symbol_for_origin(origin, substitution)
        }
    }

    /// Convert a type to a string suffix for monomorphized function names
    pub(super) fn type_to_mono_suffix(ty: &Type) -> String {
        match ty {
            Type::I64 => "I64".to_string(),
            Type::I32 => "I32".to_string(),
            Type::I16 => "I16".to_string(),
            Type::I8 => "I8".to_string(),
            Type::U64 => "U64".to_string(),
            Type::U32 => "U32".to_string(),
            Type::U16 => "U16".to_string(),
            Type::U8 => "U8".to_string(),
            Type::F64 => "F64".to_string(),
            Type::F32 => "F32".to_string(),
            Type::Bool => "Bool".to_string(),
            Type::Char => "Char".to_string(),
            Type::Str => "Str".to_string(),
            Type::Struct { id, args } | Type::Enum { id, args } if args.is_empty() => {
                format!("{}_{}", id.crate_id.0, id.local.0)
            }
            Type::Struct { id, args: generics } | Type::Enum { id, args: generics } => {
                let args: String = generics
                    .iter()
                    .map(|g| format!("_{}", Self::type_to_mono_suffix(g)))
                    .collect();
                format!("{}_{}{}", id.crate_id.0, id.local.0, args)
            }
            Type::Pointer(inner) => format!("Ptr{}", Self::type_to_mono_suffix(inner)),
            Type::Slice(inner) => format!("Slice{}", Self::type_to_mono_suffix(inner)),
            Type::Array(inner, len) => {
                // Keep bound constructor lengths and recovery identities distinct.
                match len.concrete() {
                    Some(len) => format!("Arr{}_{}", Self::type_to_mono_suffix(inner), len),
                    None => format!("Arr{}_{len:?}", Self::type_to_mono_suffix(inner)),
                }
            }
            Type::ConstValue(n) => format!("c{n}"),
            Type::Reference { inner, .. } => Self::type_to_mono_suffix(inner),
            _ => "T".to_string(),
        }
    }

    pub(super) fn receiver_mono_suffix(ty: &Type) -> String {
        match ty {
            Type::Reference { inner, .. } => Self::receiver_mono_suffix(inner),
            _ => Self::type_to_mono_suffix(ty),
        }
    }
}

#[cfg(test)]
mod tests;

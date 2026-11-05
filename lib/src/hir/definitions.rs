use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirFunctionFor<P: HirPhase> {
    pub id: DefId,
    pub name: String,
    pub generic_params: Vec<crate::types::GenericParamDecl>,
    pub generic_bounds: HirGenericBounds,
    pub params: Vec<HirParam>,
    pub ret_type: Type,
    pub body: HirBlockFor<P>,
    pub is_curried: bool,
    pub is_method: bool,
    pub self_receiver: Option<ReceiverMode>,
    pub is_unsafe: bool,
}

impl<P: HirPhase> HirFunctionFor<P> {
    /// The function's callable type: its parameter types, return type and safety.
    pub fn fn_type(&self) -> Type {
        Type::function_with_safety(
            self.params.iter().map(|param| param.ty.clone()).collect(),
            self.ret_type.clone(),
            crate::types::FunctionSafety::from_is_unsafe(self.is_unsafe),
        )
    }
}

pub type HirFunction = HirFunctionFor<UnresolvedHir>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirParam {
    pub name: String,
    pub local_id: HirLocalId,
    pub ty: Type,
    pub mutable: bool,
    pub is_ref: bool, // parameter is passed by reference
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirStruct {
    pub id: DefId,
    pub name: String,
    pub generic_params: Vec<crate::types::GenericParamDecl>,
    pub fields: Vec<HirField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirField {
    pub id: FieldId,
    pub name: String,
    pub ty: Type,
    pub public: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirEnum {
    pub id: DefId,
    pub name: String,
    pub generic_params: Vec<crate::types::GenericParamDecl>,
    pub variants: Vec<HirVariant>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirVariant {
    pub id: VariantId,
    pub name: String,
    pub fields: HirVariantFields,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HirVariantFields {
    Named(Vec<HirField>),
    Positional(Vec<Type>),
    Unit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirTraitFor<P: HirPhase> {
    pub id: DefId,
    pub name: String,
    pub generic_params: Vec<crate::types::GenericParamDecl>,
    pub target: Option<crate::types::GenericParamDecl>,
    pub predicates: Vec<crate::types::Predicate>,
    pub associated_types: Vec<HirAssociatedTypeDecl>,
    pub methods: HashMap<String, HirFunctionFor<P>>,
    pub signatures: HashMap<String, HirFunctionSig>,
}

pub type HirTrait = HirTraitFor<UnresolvedHir>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirAssociatedTypeDecl {
    pub id: AssocTypeId,
    pub name: String,
    pub kind: crate::type_services::kind::Kind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirFunctionSig {
    pub id: DefId,
    pub name: String,
    pub generic_params: Vec<crate::types::GenericParamDecl>,
    pub params: Vec<Type>,
    pub ret: Type,
    pub generic_bounds: HirGenericBounds,
    pub self_receiver: Option<ReceiverMode>,
    pub is_unsafe: bool,
}

impl HirFunctionSig {
    /// The signature's callable type: its parameter types, return type and safety.
    pub fn fn_type(&self) -> Type {
        Type::function_with_safety(
            self.params.clone(),
            self.ret.clone(),
            crate::types::FunctionSafety::from_is_unsafe(self.is_unsafe),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirImplFor<P: HirPhase> {
    pub id: DefId,
    pub owner: HirImplOwner,
    pub type_name: String,
    pub type_generics: Vec<crate::types::GenericParamDecl>,
    pub receiver_pattern: P::ImplReceiverAuthority,
    pub trait_name: Option<String>,
    pub trait_id: Option<DefId>,
    pub trait_generics: Vec<crate::types::GenericParamDecl>,
    pub trait_arg_types: Vec<Type>,
    pub associated_types: Vec<HirAssociatedTypeDef>,
    pub bounds: HirGenericBounds,
    pub methods: HashMap<String, HirFunctionFor<P>>,
}

pub type HirImpl = HirImplFor<UnresolvedHir>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HirImplOwner {
    Named(String),
    BuiltinSlice,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirAssociatedTypeDef {
    pub id: AssocTypeId,
    pub name: String,
    pub kind: crate::type_services::kind::Kind,
    pub ty: Type,
}

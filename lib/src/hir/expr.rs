use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HirClosureCapture {
    pub name: String,
    pub local_id: HirLocalId,
    pub kind: HirClosureCaptureKind,
    pub mutable: bool,
    pub ty: Type,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HirClosureCaptureKind {
    SharedBorrow,
    MutableBorrow,
    Move,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirBlockFor<P: HirPhase> {
    pub stmts: Vec<HirStmtFor<P>>,
    pub ty: Type,
}

pub type HirBlock = HirBlockFor<UnresolvedHir>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HirStmtFor<P: HirPhase> {
    Let {
        name: String,
        local_id: HirLocalId,
        ty: Type,
        value: HirExprFor<P>,
        mutable: bool,
    },
    Expr(HirExprFor<P>),
    Return(Option<HirExprFor<P>>),
    Break(Option<HirExprFor<P>>),
    Continue,
}

pub type HirStmt = HirStmtFor<UnresolvedHir>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirExprFor<P: HirPhase> {
    pub kind: HirExprKindFor<P>,
    pub ty: Type,
    pub span: Span,
}

pub type HirExpr = HirExprFor<UnresolvedHir>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirStructLiteralFieldFor<P: HirPhase> {
    pub name: String,
    pub value: HirExprFor<P>,
    pub field: Option<HirFieldLocation>,
}

pub type HirStructLiteralField = HirStructLiteralFieldFor<UnresolvedHir>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HirVarTarget {
    Local(HirLocalId),
    Function(DefId),
    Extern(DefId),
    Instance(InstanceId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HirCallTarget {
    Function(DefId),
    Extern(DefId),
    Instance(InstanceId),
    Local(HirLocalId),
    Intrinsic(String),
    StaticMethod(HirStaticMethodTarget),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HirVarRef {
    pub name: String,
    pub target: HirVarTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirStructPatternField {
    pub name: String,
    pub field: Option<HirFieldLocation>,
    pub pattern: HirPattern,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HirExprKindFor<P: HirPhase> {
    /// Integer literal
    IntLiteral(i64),
    /// Const generic parameter (`N: usize`) read as a value; monomorphization rewrites it to
    /// an `IntLiteral` of the concrete length.
    ConstParam(crate::types::GenericParamId),
    /// Float literal
    FloatLiteral(f64),
    /// Bool literal
    BoolLiteral(bool),
    /// String literal
    StringLiteral(String),
    /// Char literal
    CharLiteral(char),
    /// Array literal
    ArrayLiteral(Vec<HirExprFor<P>>),
    /// Fixed array initialized by evaluating one value and copying it `len` times
    ArrayRepeat(Box<HirExprFor<P>>, crate::types::ArrayLen),
    /// Tuple literal
    TupleLiteral(Vec<HirExprFor<P>>),
    /// Unit value
    Unit,
    /// Variable reference
    Var(String),
    /// Resolved function or extern reference
    ResolvedVar(HirVarRef),
    /// Struct field access
    FieldAccess(Box<HirExprFor<P>>, String, Option<HirFieldLocation>),
    /// Tuple index access
    TupleIndex(Box<HirExprFor<P>>, u32),
    /// Binary operation
    BinOp(BinOp, Box<HirExprFor<P>>, Box<HirExprFor<P>>),
    /// Unary operation
    UnaryOp(UnaryOp, Box<HirExprFor<P>>),
    /// Function call
    Call(
        Box<HirExprFor<P>>,
        Vec<HirExprFor<P>>,
        Option<HirCallTarget>,
    ),
    /// Method call
    MethodCall(
        Box<HirExprFor<P>>,
        String,
        Vec<HirExprFor<P>>,
        Option<ReceiverMode>,
        P::MethodAuthority,
    ),
    /// Try short-circuit expression lowered through Try and FromResidual traits
    Try {
        expr: Box<HirExprFor<P>>,
        branch_method: P::MethodAuthority,
        branch_target: Option<HirCallTarget>,
        branch_self_receiver: Option<ReceiverMode>,
        from_residual_target: P::ResidualAuthority,
        output_ty: Type,
        residual_ty: Type,
        return_ty: Type,
        control_flow_enum: DefId,
        break_variant: HirVariantLocation,
        continue_variant: HirVariantLocation,
    },
    /// Struct construction
    StructLiteral(String, Option<DefId>, Vec<HirStructLiteralFieldFor<P>>),
    /// Enum variant construction
    EnumVariant(
        String,
        String,
        Vec<HirExprFor<P>>,
        Option<HirVariantLocation>,
    ),
    /// If expression
    If {
        condition: Box<HirExprFor<P>>,
        then_branch: HirBlockFor<P>,
        else_branch: Option<HirBlockFor<P>>,
    },
    /// Match expression
    Match {
        scrutinee: Box<HirExprFor<P>>,
        arms: Vec<HirMatchArmFor<P>>,
    },
    /// While loop
    While {
        condition: Box<HirExprFor<P>>,
        body: HirBlockFor<P>,
    },
    /// For loop (desugared into while over iterator)
    For {
        var: String,
        local_id: HirLocalId,
        iter: Box<HirExprFor<P>>,
        body: HirBlockFor<P>,
    },
    /// Infinite loop
    Loop(HirBlockFor<P>),
    /// Block expression
    Block(HirBlockFor<P>),
    /// Explicit unsafe block; this marker is erased when converting to accepted HIR.
    UnsafeBlock(HirBlockFor<P>),
    /// Lambda/closure
    Lambda {
        params: Vec<HirParam>,
        body: HirBlockFor<P>,
        captures: Vec<HirClosureCapture>,
    },
    /// Reference creation
    Ref(bool, Box<HirExprFor<P>>), // mutable, expr
    /// Dereference
    Deref(Box<HirExprFor<P>>),
    /// Type cast
    Cast(Box<HirExprFor<P>>, Type),
    /// Assignment (for mutable variables, field assignment)
    Assign(Box<HirExprFor<P>>, Box<HirExprFor<P>>),
    /// Compiler intrinsic (maps directly to LLVM operation)
    /// e.g., ~I64Add, ~F64Mul
    Intrinsic {
        name: String,
        args: Vec<HirExprFor<P>>,
    },
}

pub type HirExprKind = HirExprKindFor<UnresolvedHir>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

impl BinOp {
    pub fn from_str(s: &str) -> Option<BinOp> {
        // All operators are now handled via std infix declarations and method calls
        // Short-circuit operators (&&, ||) are kept as BinOp for special handling
        match s {
            "&&" => Some(BinOp::And),
            "||" => Some(BinOp::Or),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HirMatchArmFor<P: HirPhase> {
    pub pattern: HirPattern,
    pub guard: Option<HirExprFor<P>>,
    pub body: HirBlockFor<P>,
}

pub type HirMatchArm = HirMatchArmFor<UnresolvedHir>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HirPattern {
    /// Wildcard _
    Wildcard,
    /// Bind a name
    Binding {
        name: String,
        local_id: HirLocalId,
        mutable: bool,
    },
    /// Literal pattern
    Literal(HirLiteralPattern),
    /// Tuple pattern
    Tuple(Vec<HirPattern>),
    /// Struct pattern
    Struct(String, Option<DefId>, Vec<Type>, Vec<HirStructPatternField>),
    /// Enum variant pattern
    Enum(String, String, Option<HirVariantLocation>, Vec<HirPattern>),
    /// Or pattern (not yet used)
    Or(Vec<HirPattern>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HirLiteralPattern {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Char(char),
}

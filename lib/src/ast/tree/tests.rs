use super::*;

macro_rules! impl_test_to_string_with_format_context {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl $ty {
                pub fn to_string(&self) -> String {
                    let mut context = crate::fmt::FormatContext::new();
                    let mut output = String::new();
                    crate::fmt::FormatNode::fmt_with(self, &mut context, &mut output)
                        .expect("formatting into a String should not fail");
                    output
                }
            }
        )+
    };
}

impl_test_to_string_with_format_context!(
    ModuleDecl,
    TopLevel,
    StructDecl,
    StructDeclField,
    EnumDecl,
    EnumVariant,
    NamedFieldsOrTypesList,
    TraitDecl,
    Impl,
    Path,
    IdentifierPath,
    TypePath,
    IdentOrType,
    Ident,
    ParseType,
    ParseTypeInner,
    GenericParamDecl,
    TypeApplication,
    TypeLambda,
    TypeHole,
    WhereClause,
    FunctionSig,
    FunctionDecl,
    LambdaDecl,
    Statement,
    Assignment,
    AssignmentLHS,
    Expression,
    UnaryExpr,
    Operator,
    PrimaryExpr,
    Operand,
    Match,
    MatchArm,
    Tuple,
    NativeOperator,
    SecondaryExpr,
    IdentOrNumber,
    Argument,
    Literal,
    Instance,
    Condition,
    If,
    Else,
    Loop,
    Array,
    MacroDecl,
    MacroEntry,
    MacroFragment,
    MacroInvoc,
    Pattern,
    PatternKind,
    IdentPattern,
    ArrayPattern,
    InstancePattern,
    FieldsPatternOrArgumentsPattern,
    FieldPattern,
);

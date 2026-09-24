use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{
    language_items::LanguageItemRole,
    lexer::{Span, Token},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Program {
    pub module: Module,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleDecl(pub Module);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Module {
    pub name: Option<Ident>,
    pub top_levels: Vec<TopLevel>,
    pub is_inline: bool,
    pub filepath: Option<PathBuf>,
}

impl Module {
    pub fn top_level_from_ident(&self, ident: &str) -> Option<&TopLevel> {
        self.top_levels
            .iter()
            .find(|tl| tl.get_ident().map(|i| i == ident).unwrap_or(false))
    }

    pub fn has_macro_invoc(&self) -> bool {
        self.top_levels.iter().any(|tl| match &tl {
            TopLevel::MacroInvoc(_) => true,
            _ => false,
        })
    }
}

// Used to parse the first module without a name
#[derive(Clone, Serialize, Deserialize)]
pub struct ModuleInner {
    pub top_levels: Vec<TopLevel>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TopLevel {
    Module(ModuleDecl),
    Mod(Ident, bool), // mod name; - loads a .rk file (bool = exported)
    Import(Path),
    GlobImport(Vec<String>), // > module::path::* - import all exported items into scope
    Export(Path),
    GlobExport(Vec<String>), // < module::path::* — re-export all items from a module
    Extern(FunctionSig),
    InfixOperator(u8, String),
    MacroDecl(MacroDecl),
    MacroInvoc(MacroInvoc),
    FunctionSig(FunctionSig),
    FunctionDecl(FunctionDecl),
    StructDecl(StructDecl),
    TraitDecl(TraitDecl),
    EnumDecl(EnumDecl),
    Impl(Impl),
    NewType(ParseTypeInner, ParseType),
}

impl TopLevel {
    pub fn get_ident(&self) -> Option<&String> {
        match self {
            TopLevel::FunctionDecl(fd) => Some(&fd.name.name),
            TopLevel::StructDecl(sd) => Some(&sd.name.name),
            TopLevel::TraitDecl(td) => Some(&td.name.name),
            TopLevel::EnumDecl(ed) => Some(&ed.name.name),
            TopLevel::Impl(i) => Some(&i.name.name),
            TopLevel::MacroDecl(md) => Some(&md.name.name),
            TopLevel::MacroInvoc(mi) => Some(&mi.name.name),
            TopLevel::FunctionSig(fs) => Some(&fs.name.name),
            TopLevel::NewType(nt, _) => Some(&nt.name),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructDecl {
    pub name: ParseTypeInner,
    pub generic_params: Vec<GenericParamDecl>,
    pub fields: Vec<StructDeclField>,
    pub exported: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructDeclField {
    pub name: Ident,
    pub ty: ParseType,
    pub public: bool,
    pub default: Option<Expression>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TraitDecl {
    pub name: ParseTypeInner,
    pub generic_params: Vec<GenericParamDecl>,
    pub for_: Option<GenericParamDecl>,
    pub where_clauses: Vec<WhereClause>,
    pub associated_types: Vec<AssociatedTypeDecl>,
    pub methods: HashMap<Ident, FunctionDecl>,
    pub signatures: HashMap<Ident, FunctionSig>,
    pub exported: bool,
    pub language_items: LanguageItemAnnotations,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssociatedTypeDecl {
    pub name: Ident,
    pub kind: Option<TypeApplication>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnumDecl {
    pub name: ParseTypeInner,
    pub variants: Vec<EnumVariant>,
    pub exported: bool,
    pub language_items: LanguageItemAnnotations,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LanguageItemMarker {
    pub role: LanguageItemRole,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LanguageItemMemberKind {
    AssociatedType,
    Method,
    Variant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LanguageItemMemberMarker {
    pub marker: LanguageItemMarker,
    pub kind: LanguageItemMemberKind,
    pub member_name: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LanguageItemAnnotations {
    pub root: Option<LanguageItemMarker>,
    pub members: Vec<LanguageItemMemberMarker>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnumVariant {
    pub name: ParseTypeInner,
    pub fields: NamedFieldsOrTypesList,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NamedFieldsOrTypesList {
    NamedFields(Vec<StructDeclField>),
    TypesList(Vec<ParseType>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Impl {
    pub name: ParseTypeInner,
    pub for_: Option<ParseType>,
    pub associated_types: Vec<AssociatedTypeDef>,
    pub methods: HashMap<Ident, FunctionDecl>,
    pub signatures: HashMap<Ident, FunctionSig>,
    /// Where clauses for trait bounds, e.g., `where T: Show`
    pub where_clauses: Vec<WhereClause>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssociatedTypeDef {
    pub name: Ident,
    pub kind: Option<TypeApplication>,
    pub ty: ParseType,
}

/// A where clause constraining a type parameter, e.g., `T: Show`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhereClause {
    pub subject: ParseType,
    pub trait_bound: Option<ParseType>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ParseType {
    Function(Vec<ParseType>),
    Type(ParseTypeInner),
    Application(TypeApplication),
    Lambda(TypeLambda),
    Hole(TypeHole),
    Associated {
        base: ParseTypeInner,
        member: Ident,
    },
    Slice(Box<ParseType>),
    Array {
        inner: Box<ParseType>,
        len: usize,
    },
    Tuple(Vec<ParseType>),
    Reference {
        is_mut: bool,
        pointee: Box<ParseType>,
    },
    Pointer(Box<ParseType>),
    Unit(Span),
}

impl ParseType {
    pub fn span(&self) -> Span {
        match self {
            ParseType::Type(inner) => inner.span.clone(),
            ParseType::Application(application) => application.span.clone(),
            ParseType::Lambda(lambda) => lambda.span.clone(),
            ParseType::Hole(hole) => hole.span.clone(),
            ParseType::Associated { base, member } => Span::new(
                base.span.file_path.clone(),
                base.span.start,
                member.span.end,
            ),
            ParseType::Slice(inner)
            | ParseType::Array { inner, .. }
            | ParseType::Reference { pointee: inner, .. }
            | ParseType::Pointer(inner) => inner.span(),
            ParseType::Tuple(types) | ParseType::Function(types) => types
                .first()
                .expect("compound parse type must retain a source element")
                .span(),
            ParseType::Unit(span) => span.clone(),
        }
    }

    /// Get the type name as a string for use in method lookups
    pub fn type_name(&self) -> String {
        match self {
            ParseType::Type(inner) => inner.name.clone(),
            ParseType::Application(application) => application.constructor.type_name(),
            ParseType::Lambda(_) => "<type lambda>".to_string(),
            ParseType::Hole(_) => "_".to_string(),
            ParseType::Associated { base, member } => format!("{}::{}", base.name, member.name),
            ParseType::Reference { is_mut, pointee } => {
                if *is_mut {
                    format!("&mut {}", pointee.type_name())
                } else {
                    format!("&{}", pointee.type_name())
                }
            }
            ParseType::Pointer(inner) => format!("*{}", inner.type_name()),
            ParseType::Slice(inner) => format!("[{}]", inner.type_name()),
            ParseType::Array { inner, len } => format!("[{}; {}]", inner.type_name(), len),
            ParseType::Tuple(elems) => {
                let inner: Vec<String> = elems.iter().map(|e| e.type_name()).collect();
                format!("({})", inner.join(", "))
            }
            ParseType::Function(args) => args
                .iter()
                .map(|a| a.type_name())
                .collect::<Vec<_>>()
                .join(" -> "),
            ParseType::Unit(_) => "()".to_string(),
        }
    }

    /// Get the generics for this type (only meaningful for ParseType::Type)
    pub fn generics(&self) -> &[ParseType] {
        match self {
            ParseType::Type(inner) => &inner.generics,
            ParseType::Application(application) => &application.args,
            _ => &[],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParseTypeInner {
    pub name: String,
    pub generics: Vec<ParseType>,
    pub span: Span,
}

/// A generic binder introduced by a declaration or explicit type lambda.
///
/// `kind` is syntax only: its application contains the source holes that make
/// a constructor binder explicit, but no kind is inferred here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenericParamDecl {
    pub name: Ident,
    pub kind: Option<TypeApplication>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeApplication {
    pub constructor: Box<ParseType>,
    pub args: Vec<ParseType>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeLambda {
    pub params: Vec<GenericParamDecl>,
    pub body: Box<ParseType>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypeHole {
    pub span: Span,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct MacroDecl {
    pub name: Ident,
    pub entries: Vec<MacroEntry>,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct MacroEntry {
    pub defs: Vec<MacroFragment>,
    pub body: Vec<MacroFragment>,
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub enum MacroFragment {
    Ident(Ident),
    Expr(Ident),
    Type(Ident),
    Token(Token),
    Repetition(Vec<MacroFragment>),
}

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub struct MacroInvoc {
    pub name: Ident,
    pub args: Vec<Token>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionSig {
    pub name: Ident,
    pub sig: ParseType,
    pub where_clauses: Vec<WhereClause>,
    pub self_receiver: Option<SelfReceiverMode>,
    pub is_unsafe: bool,
    pub exported: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionDecl {
    pub name: Ident,
    pub lambda: LambdaDecl,
    pub self_receiver: Option<SelfReceiverMode>,
    pub is_unsafe: bool,
    pub exported: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SelfReceiverMode {
    Shared,
    Mut,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LambdaArrowKind {
    Normal,
    Unit,
    Curried,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LambdaDecl {
    pub parameters: Vec<Pattern>,
    pub body: Block,
    pub arrow_kind: LambdaArrowKind,
    /// Span of the lambda introducer arrow (or the preserved shorthand span).
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub statements: Vec<Statement>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Statement {
    Assignment(Assignment),
    Expression(Expression),
    Return(Option<Expression>),
    Continue(Option<Expression>),
    Break(Option<Expression>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AssignmentLHS {
    Expression(UnaryExpr),
    Pattern {
        pattern: Pattern,
        type_annotation: Option<ParseType>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    pub lhs: AssignmentLHS,
    pub rhs: Expression,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Expression {
    BinopExpr(UnaryExpr, Operator, Box<Expression>),
    UnaryExpr(UnaryExpr),
    CastExpr(Box<Expression>, ParseType),
    Range(RangeExpr),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RangeExpr {
    pub start: Option<Box<Expression>>,
    pub end: Option<Box<Expression>>,
    pub inclusive: bool,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UnaryExpr {
    PrimaryExpr(PrimaryExpr),
    UnaryExpr(Operator, Box<UnaryExpr>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrimaryExpr {
    pub operand: Operand,
    pub secondaries: Option<Vec<SecondaryExpr>>,
    pub type_annotation: Option<ParseType>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Operand {
    Literal(Literal),
    Ident(IdentifierPath),
    CallHole(Span),
    /// Ident prefixed with a @ are desugared to self.ident
    SelfIdent(Ident),
    Instance(Instance),
    LambdaDecl(LambdaDecl),
    Tuple(Tuple),
    NativeOperator(NativeOperator),
    If(Box<If>),
    Match(Box<Match>),
    Loop(Box<Loop>),
    Unsafe(Block, Span),
    Expression(Box<Expression>), // parenthesis
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Match {
    pub expr: Expression,
    pub arms: Vec<MatchArm>,
    /// Original parser sugar for formatting only; semantic visitors use the expanded arms.
    #[serde(skip)]
    pub do_syntax: Option<Vec<DoStatement>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DoStatement {
    Bind(Pattern, Expression, Span),
    Statement(Statement, Span),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub condition: Option<Expression>,
    pub body: Block,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pattern {
    pub binding: Option<Ident>,
    pub kind: PatternKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PatternKind {
    Ident(IdentPattern),
    Literal(Literal),
    Tuple(Vec<Pattern>),
    Array(Vec<ArrayPattern>),
    Instance(InstancePattern),
    Nested(Box<Pattern>), // parenthesis
    Wildcard,
    Reference {
        pattern: Box<Pattern>,
        mutable: bool,
    }, // &pattern or &mut pattern
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ArrayPattern {
    Pattern(Pattern),
    Rest(IdentPattern),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdentPattern {
    pub name: Ident,
    pub mut_: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstancePattern {
    pub name: TypePath,
    pub args: FieldsPatternOrArgumentsPattern,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FieldsPatternOrArgumentsPattern {
    Fields(Vec<FieldPattern>),
    Arguments(Vec<Pattern>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldPattern {
    pub name: Ident,
    pub pattern: Pattern,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tuple {
    pub elements: Vec<Expression>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeOperator {
    pub name: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct If {
    pub condition: Condition,
    pub then: Block,
    pub else_: Option<Else>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Condition {
    pub pattern: Option<Pattern>,
    pub expression: Expression,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Else {
    If(Box<If>),
    Block(Block),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Loop {
    While(Condition, Block, Span),
    For(Pattern, Expression, Block, Span),
    Loop(Block, Span),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    pub name: TypePath,
    pub fields: HashMap<Ident, Expression>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IdentOrType {
    Ident(Ident),
    Type(ParseType),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Path {
    Ident(IdentifierPath),
    Type(TypePath),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdentifierPath {
    pub path: Vec<IdentOrType>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TypePath {
    pub path: Vec<IdentOrType>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Literal {
    pub kind: LiteralKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LiteralKind {
    Bool(bool),
    Number(u64),
    Float(f64),
    Array(Array),
    ArrayRepeat { value: Box<Expression>, len: usize },
    String(String),
    Char(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Array {
    pub elements: Vec<Expression>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IdentOrNumber {
    Ident(Ident),
    Number(u64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SecondaryExpr {
    Arguments(Vec<Argument>),
    Indice(Box<Expression>), // Boxing here to keep the enum size low
    Dot(IdentOrNumber),
    Interogation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Argument {
    pub arg: Expression,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ident {
    pub name: String,
    pub span: Span,
}

impl PartialEq for Ident {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for Ident {}

impl std::hash::Hash for Ident {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

impl PartialOrd for Ident {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.name.cmp(&other.name))
    }
}

impl Ord for Ident {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.name.cmp(&other.name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Number {
    pub value: String,
    pub span: Span,
}

impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl Eq for Number {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operator {
    pub value: String,
    pub span: Span,
}

impl PartialEq for Operator {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl Eq for Operator {}

#[cfg(test)]
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

#[cfg(test)]
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

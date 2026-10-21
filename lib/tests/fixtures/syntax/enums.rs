pub enum Fields {
    Nullary,
    Named(&'static [(&'static str, &'static str, Option<&'static str>)]),
    Positional(&'static [&'static str]),
}

pub struct Variant {
    pub name: &'static str,
    pub fields: Fields,
}

pub struct EnumCase {
    pub name: &'static str,
    pub source: &'static str,
    pub rust_accept: bool,
    // Highlight a unique/last spelling (empty means EOF); related is an opener.
    pub error: Option<(&'static str, &'static str, Option<&'static str>)>,
    pub variants: &'static [Variant],
}

macro_rules! v {
    ($name:literal) => { Variant { name: $name, fields: Fields::Nullary } };
    ($name:literal; fields $fields:expr) => { Variant { name: $name, fields: Fields::Named($fields) } };
    ($name:literal; $($ty:literal),+) => { Variant { name: $name, fields: Fields::Positional(&[$($ty),+]) } };
}
macro_rules! yes {
    ($name:literal, $source:literal, $variants:expr) => {
        EnumCase { name: $name, source: $source, rust_accept: true, error: None, variants: $variants }
    };
}
macro_rules! no {
    ($name:literal, $source:literal, $rust:literal, $code:literal, $highlight:literal) => {
        EnumCase { name: $name, source: $source, rust_accept: $rust, error: Some(($code, $highlight, None)), variants: &[] }
    };
}

pub const CASES: &[EnumCase] = &[
    yes!("empty-enum", "enum Empty\n", &[]),
    yes!("eof-enum", "enum Empty", &[]),
    yes!("nullary", "enum Flag\n    On\n    Off\n", &[v!("On"), v!("Off")]),
    yes!("positional", "enum Choice T, {E = T}\n    Empty\n    Some T\n    Pair T, E\n", &[v!("Empty"), v!("Some"; "T"), v!("Pair"; "T", "E")]),
    yes!("named", "< enum Record T\n    Named\n        < value: T\n        count: I64 = 7\n    Empty\n", &[v!("Named"; fields &[("value", "T", None), ("count", "I64", Some("7"))]), v!("Empty")]),
    yes!("all-shapes", "enum Mixed T\n    Empty\n    One T\n    Named\n        value: T\n    Last\n", &[v!("Empty"), v!("One"; "T"), v!("Named"; fields &[("value", "T", None)]), v!("Last")]),
    yes!("two-space", "enum E\n  Named\n    field: T\n  Last", &[v!("Named"; fields &[("field", "T", None)]), v!("Last")]),
    yes!("nested-margin", "    enum Choice T\n        One T\n        Named\n            < field: T\n", &[v!("One"; "T"), v!("Named"; fields &[("field", "T", None)])]),
    yes!("application-owns-comma", "enum E\n    One Result T, E\n", &[v!("One"; "Result T, E")]),
    yes!("grouped-siblings", "enum E\n    Pair (Result T, E), U\n", &[v!("Pair"; "(Result T, E)", "U")]),
    yes!("grouped-adjacent", "enum E\n    Pair (T) U\n", &[v!("Pair"; "(T)", "U")]),
    yes!("ungrouped-adjacent", "enum E\n    One T U\n", &[v!("One"; "T U")]),
    yes!("tuple-payload", "enum E\n    One (T, U)\n", &[v!("One"; "(T, U)")]),
    yes!("unit-payload", "enum E\n    One ()\n", &[v!("One"; "()")]),
    yes!("arrow-payload", "enum E\n    One A -> B -> C\n", &[v!("One"; "A -> B -> C")]),
    yes!("grouped-functions", "enum E\n    Pair (A -> B) (C -> D)\n", &[v!("Pair"; "(A -> B)", "(C -> D)")]),
    yes!("array-reference", "enum E\n    Pair [T; 4], &mut U\n", &[v!("Pair"; "[T; 4]", "&mut U")]),
    yes!("trailing-comma", "enum E\n    One T,\n", &[v!("One"; "T")]),
    yes!("constructor-section", "enum E (F _), T\n    One (F _)\n    Named\n        field: F T\n", &[v!("One"; "(F _)"), v!("Named"; fields &[("field", "F T", None)])]),
    yes!("explicit-kind", "enum E (H: (Type -> Type) -> Type)\n    Empty\n", &[v!("Empty")]),
    yes!("named-default-block", "enum E\n    Named\n        field: Packet = Packet\n            value: 7\n    Last\n", &[v!("Named"; fields &[("field", "Packet", Some("Packet\n            value: 7"))]), v!("Last")]),
    yes!("quoted-variant-lookalike", "enum E\n    Named\n        field: String = \"first\n    Fake\nlast\".clone!\n    Last\n", &[v!("Named"; fields &[("field", "String", Some("\"first\n    Fake\nlast\".clone!"))]), v!("Last")]),
    yes!("comments-crlf", "// lead\r\n< enum E T\r\n    // 雪\r\n    One /* é */ T // tail\r\n\r\n    Named\r\n        < value: T = make 7 /* tail */\r\n", &[v!("One"; "T"), v!("Named"; fields &[("value", "T", Some("make 7"))])]),
    yes!("duplicate-is-syntax", "enum E\n    Same\n    Same T\n", &[v!("Same"), v!("Same"; "T")]),
    no!("lower-variant", "enum E\n    lower\n", false, "variant", "lower"),
    no!("variant-visibility", "enum E\n    < One\n", false, "variant", "<"),
    no!("missing-name", "enum\n", false, "declaration-name", ""),
    no!("lower-declaration", "enum lower\n", false, "declaration-name", "lower"),
    no!("discriminant", "enum E\n    One = 1\n", false, "variant-end", "="),
    no!("empty-comma", "enum E\n    One,\n", false, "variant-end", ","),
    no!("double-comma", "enum E\n    One T,,\n", false, "variant-end", ","),
    no!("type-tail", "enum E\n    One T + U\n", false, "variant-end", "+"),
    no!("declaration-kind-not-argument", "enum E\n    One (F: Type -> Type)\n", false, "type-close", ":"),
    no!("newline-positional", "enum E\n    One\n        T\n", false, "field", "T"),
    no!("missing-field-colon", "enum E\n    One\n        field T\n", false, "field-colon", "T"),
    no!("named-margin", "enum E\n    One\n      field: T\n", false, "declaration-indent", "field"),
    no!("variant-margin", "enum E\n    One\n  Two\n", false, "declaration-indent", "Two"),
    no!("trailing-declaration", "enum E\n    One\nstruct Other\n", false, "declaration-indent", "struct"),
    no!("mixed-shape", "enum E\n    One T\n        field: U\n", true, "variant-shape", "field"),
    no!("unsupported-lambda-payload", "enum E\n    One \\T -> T\n", true, "variant-end", "\\"),
    no!("unsupported-marker", "enum E\n    lang invalid\n    One\n", false, "variant", "lang"),
];

pub struct StructCase {
    pub name: &'static str,
    pub source: &'static str,
    pub rust_accept: bool,
    pub error: Option<(&'static str, usize, usize, i64)>,
    pub fields: &'static [(&'static str, &'static str, Option<&'static str>)],
}

macro_rules! yes {
    ($name:literal, $source:literal, $fields:expr) => {
        StructCase { name: $name, source: $source, rust_accept: true, error: None, fields: $fields }
    };
}
macro_rules! no {
    ($name:literal, $source:literal, $rust:literal, $code:literal, $start:literal, $end:literal, $related:literal) => {
        StructCase { name: $name, source: $source, rust_accept: $rust, error: Some(($code, $start, $end, $related)), fields: &[] }
    };
}

pub const CASES: &[StructCase] = &[
    yes!("empty", "struct Empty\n", &[]),
    yes!("eof", "struct Empty", &[]),
    yes!("public", "< struct Empty // tail\r\n\r\n", &[]),
    yes!("fields", "struct Pair\n    first: I64\n    < second: Bool\n", &[("first", "I64", None), ("second", "Bool", None)]),
    yes!("two-space", "struct Pair\n  first: I64\n  second: Bool", &[("first", "I64", None), ("second", "Bool", None)]),
    yes!("generics", "struct Pair A, {B = A}\n    < first: A\n    < second: B\n", &[("first", "A", None), ("second", "B", None)]),
    yes!("constructor", "struct Higher (F _), T\n    value: F T\n", &[("value", "F T", None)]),
    yes!("explicit-kind", "struct Higher (H: (Type -> Type) -> Type)\n    value: I64\n", &[("value", "I64", None)]),
    yes!("default-constructor", "struct Higher T, {F _ = Result T, _}\n    value: F T\n", &[("value", "F T", None)]),
    yes!("nested-types", "struct Types T, E\n    value: Result (T, &mut E), [T; 4]\n    slice: &[T]\n    pointer: *T\n    callback: T -> (E -> T)\n", &[("value", "Result (T, &mut E), [T; 4]", None), ("slice", "&[T]", None), ("pointer", "*T", None), ("callback", "T -> (E -> T)", None)]),
    yes!("defaults", "struct Defaults\n    count: I64 = 7\n    values: [I64; 2] = [1, 2]\n    text: String = \"é雪\"\n", &[("count", "I64", Some("7")), ("values", "[I64; 2]", Some("[1, 2]")), ("text", "String", Some("\"é雪\""))]),
    yes!("default-lambda", "struct Function\n    callback: I64 -> I64 = x -> x\n", &[("callback", "I64 -> I64", Some("x -> x"))]),
    yes!("default-block-lambda", "struct Function\n    callback: () -> I64 = ->\n        7\n    other: I64\n", &[("callback", "() -> I64", Some("->\n        7")), ("other", "I64", None)]),
    yes!("default-instance", "struct Config\n    entry: Packet = Packet\n        value: 7\n    fallback: I64 = 9\n", &[("entry", "Packet", Some("Packet\n        value: 7")), ("fallback", "I64", Some("9"))]),
    yes!("macro-default", "struct Value\n    value: I64 = %identity 7\n", &[("value", "I64", Some("%identity 7"))]),
    yes!("eof-default", "struct Value\n    value: I64 = 7", &[("value", "I64", Some("7"))]),
    yes!("quoted-field-lookalike", "struct Text\n    text: String = \"first\n    fake: T\nlast\"\n    next: I64\n", &[("text", "String", Some("\"first\n    fake: T\nlast\"")), ("next", "I64", None)]),
    yes!("multiline-token-tail", "struct Text\n    text: String = \"first\n    fake: T\nlast\".clone!\n    next: I64\n", &[("text", "String", Some("\"first\n    fake: T\nlast\".clone!")), ("next", "I64", None)]),
    yes!("type-and-default-trivia", "/* lead */\n< struct Packet T\r\n    // 雪\r\n    < value: Vec /* é */ T = make (7, 8) /* end */\r\n\r\n", &[("value", "Vec /* é */ T", Some("make (7, 8)"))]),
    yes!("nested-margin", "    < struct Packet T\n        < value: T\n        next: T\n", &[("value", "T", None), ("next", "T", None)]),
    yes!("duplicate-field-is-syntax", "struct Pair\n    same: I64\n    same: Bool\n", &[("same", "I64", None), ("same", "Bool", None)]),
    yes!("unknown-is-syntax", "struct Shape {T = Unknown}\n    value: NotResolved T\n", &[("value", "NotResolved T", None)]),
    no!("unsupported-declaration", "trait Empty\n", true, "declaration", 0, 5, -1),
    no!("empty-input", "", false, "declaration", 0, 0, -1),
    no!("missing-name", "struct\n", false, "declaration-name", 6, 6, -1),
    no!("lower-name", "struct lower\n", false, "declaration-name", 7, 12, -1),
    no!("header-field", "struct S value: T\n", false, "binder", 9, 14, -1),
    no!("application-binder", "struct S F T\n", false, "header-end", 11, 12, -1),
    no!("where-header", "struct S T where T: Show\n", false, "header-end", 11, 16, -1),
    no!("upper-field", "struct S\n    Value: T\n", false, "field", 13, 18, -1),
    no!("missing-colon", "struct S\n    value T\n", false, "field-colon", 19, 20, -1),
    no!("missing-type", "struct S\n    value:\n", false, "type", 19, 19, -1),
    no!("missing-default", "struct S\n    value: T =\n", false, "field-default", 23, 23, -1),
    no!("bad-type-tail", "struct S\n    value: T + U\n", false, "field-end", 22, 23, -1),
    no!("two-fields-one-line", "struct S\n    value: T other: U\n", false, "field-end", 22, 27, -1),
    no!("unindented-field", "struct S\nvalue: T\n", false, "declaration-indent", 9, 14, -1),
    no!("odd-indent", "struct S\n   value: T\n", false, "declaration-indent", 12, 17, -1),
    no!("sibling-margin", "struct S\n    first: T\n  other: U\n", false, "declaration-indent", 24, 29, -1),
    no!("type-continuation", "struct S\n    first: T\n        other: U\n", false, "declaration-boundary", 30, 35, -1),
    no!("following-declaration", "struct S\n    value: T\nstruct Other\n", false, "declaration-indent", 22, 28, -1),
    no!("missing-close", "struct S\n    value: [T\n", false, "unclosed", 20, 21, -1),
    no!("wrong-close", "struct S\n    value: (T]\n", false, "mismatch", 22, 23, 20),
    no!("unsupported-type-lambda", "struct S\n    value: \\T -> T\n", true, "unsupported-type", 20, 21, -1),
    no!("unsupported-tabs", "struct S\n\t\tvalue: T\n", true, "declaration-indent", 11, 16, -1),
    no!("unsupported-inline-comment", "struct S\n    /* lead */ value: T\n", false, "declaration-boundary", 24, 29, -1),
];

# Syntax at a Glance

This chapter is a reference map of the active Rock syntax. It separates the
notation used to describe syntax from programs that can be copied into a
source file.

## Reading grammar notation

Grammar descriptions in this chapter use a small notation:

- `::=` means "is defined as".
- A word written in `UPPER_CASE` is a metavariable supplied by the grammar.
- Text in double quotes is a literal token.
- `|` separates alternatives.
- A suffix `?` means zero or one occurrence.
- A suffix `*` means zero or more occurrences.
- A suffix `+` means one or more occurrences.
- `INDENT` and `DEDENT` are layout tokens produced by the lexer.

These marks describe grammar productions; they are not Rock source. Every
`rock` fence below is a complete example with its declarations in the same
fence. No grammar production uses an omission marker.

The condensed productions reuse a few base names throughout the chapter:

- `IDENTIFIER` is a lowercase or underscore-led source name; `TYPE_NAME` is an uppercase-led type name.
- `DIGIT` is one decimal digit, `CHARACTER` is one source character accepted by the literal lexer, and `OPERATOR` is one or more operator characters.
- `PATH` and `TYPE_PATH` are `::`-separated identifier or type segments.
- `EXPRESSION` means any expression form described in this chapter; `STATEMENT` is an expression, binding, assignment, `return`, `break`, or `continue` in a block.
- `BODY` is either one expression after an arrow or an indented `BLOCK`; `DECLARATION` is any top-level declaration production.
- A name ending in `_LIST` is a comma-separated sequence of the named element. `PARAMETER_LIST` is a comma-separated pattern list, `ARGUMENT_LIST` is a comma-separated expression list, and `TYPE_PARAMETER_LIST` is a comma-separated type-name list.
- `MACRO_PATTERN`, `MACRO_TEMPLATE`, and `MACRO_ARGUMENT` are token sequences interpreted by the declarative macro matcher; their typed captures are defined in the macro production below.

These are metavariables, not words that appear literally in Rock source.

The top-level shape is:

```text
PROGRAM ::= TOP_LEVEL_ITEM*
TOP_LEVEL_ITEM ::= IMPORT | MODULE | EXPORT | TYPE_DECLARATION | TRAIT_DECLARATION | IMPLEMENTATION | FUNCTION_DECLARATION | EXTERN_DECLARATION | MACRO_DECLARATION | OPERATOR_DECLARATION
```

## Layout and comments

Rock uses indentation to delimit blocks. The first indented block establishes
the file's indentation step; use that step consistently. A block starts after
a declaration or control-flow header and ends when the indentation returns to
the surrounding level.

```text
BLOCK ::= INDENT STATEMENT+ DEDENT
IF_EXPRESSION ::= "if" EXPRESSION BLOCK ("else" (BLOCK | IF_EXPRESSION))?
```

This is a complete layout example:

```rock
main = !->
    if true
        "the condition is true".println!
    else
        "the condition is false".println!
```

Line comments begin with `//`. Block comments begin with `/*` and end with
`*/`; both forms are ignored by the parser.

```rock
main = !->
    // This comment ends at the line break.
    /* This comment spans one complete comment token. */
    "comments do not produce values".println!
```

## Names and keywords

Identifiers name declarations and local bindings. Type names conventionally
start with an uppercase letter, while functions and bindings conventionally
start with a lowercase letter. The lexer reserves the following keywords:

```text
KEYWORD ::= "struct" | "enum" | "trait" | "impl" | "if" | "then" | "else" | "for" | "in" | "while" | "loop" | "macro" | "mod" | "extern" | "match" | "unsafe" | "infix" | "return" | "continue" | "break" | "type" | "mut" | "where" | "as" | "true" | "false"
```

Declaration punctuation has its own meaning:

- `<` exports a declaration, field, module, or path.
- `>` imports a path.
- `@`, `^@`, and `~@` describe shared, mutable, and consuming receivers.
- `%name` invokes a macro and `$name` names a macro fragment.

## Literals

The literal forms are:

```text
INTEGER_LITERAL ::= DIGIT+
FLOAT_LITERAL ::= DIGIT+ "." DIGIT+
BOOLEAN_LITERAL ::= "true" | "false"
CHAR_LITERAL ::= "'" CHARACTER "'"
STRING_LITERAL ::= "\"" CHARACTER* "\""
ARRAY_LITERAL ::= "[" EXPRESSION_LIST? "]" | "[" EXPRESSION ";" INTEGER_LITERAL "]"
TUPLE_LITERAL ::= "(" EXPRESSION "," EXPRESSION_LIST? ")"
RANGE_EXPRESSION ::= EXPRESSION? (".." | "..=") EXPRESSION?
```

An inclusive range written with `..=` requires an end expression. Native ranges are first-class values and can be used for slicing, for example `&values[1..3]`, `&values[..count]`, and `&values[..]`.

This program constructs integer, floating-point, Boolean, character, string,
array, and tuple literals, with explicit annotations for their types:

```rock
main = !->
    count: I64 = 42
    ratio: F64 = 3.5
    enabled: Bool = true
    initial: Char = 'R'
    greeting: &Str = "Rock"
    values: [I64; 3] = [1, 2, 3]
    repeated: [I64; 3] = [7; 3]
    pair: (I64, &Str) = (1, "one")
    count.println!
    ratio.println!
    enabled.println!
    initial.println!
    greeting.println!
    values.println!
    repeated.println!
    pair.0.println!
    pair.1.println!
```

Strings and characters are byte-oriented in the current implementation. Use
ordinary ASCII escapes only when targeting the current parser and verify
non-ASCII behavior against the target compiler.

## Bindings and places

The grammar distinguishes a binding from an assignment target:

```text
BINDING ::= "mut"? IDENTIFIER (":" TYPE)? "=" EXPRESSION
ASSIGNMENT ::= PLACE "=" EXPRESSION
PLACE ::= IDENTIFIER | PLACE "." IDENTIFIER | PLACE "[" EXPRESSION "]" | "(" PLACE "," PLACE_LIST ")"
```

The following complete program declares every place before assigning through
it:

```rock
main = !->
    mut first: I64 = 1
    second: I64 = 2
    first = second + 3
    mut values: [I64; 2] = [first, second]
    values[1] = first
    first.println!
    values[1].println!
```

`mut` permits mutable borrowing and mutable receiver calls. Ordinary local
reassignment is available for a binding whose inferred type does not change.

## Functions and calls

Function signatures and definitions use arrows. Multiple parameter arrows are
curried type notation; a definition with comma-separated parameters receives
those parameters in one call expression.

```text
FUNCTION_SIGNATURE ::= IDENTIFIER ":" TYPE
FUNCTION_DEFINITION ::= IDENTIFIER "=" PARAMETER_LIST ARROW BODY
ARROW ::= "->" | "!->" | "~>"
CALL ::= EXPRESSION ARGUMENT_LIST | EXPRESSION "!"
```

This program shows value-returning, unit-arrow, and zero-argument calls:

```rock
add: I64 -> I64 -> I64
add = left, right -> left + right

announce: &Str -> ()
announce = message !->
    message.println!

main = !->
    total: I64 = add 2, 3
    announce "five"
    total.println!
```

Arguments are separated by commas, and each argument consumes a complete
expression. A nested call owns its comma-separated arguments without extra
grouping; parentheses do not turn a call into Rust-style
`function(arguments)` syntax.

In an inline argument list, a bare lambda has at most one parameter; commas
separate call arguments. Group a multi-parameter lambda explicitly:
`apply (left, right -> left + right)`.

```rock
add: I64 -> I64 -> I64
add = left, right -> left + right

double: I64 -> I64
double = value -> value * 2

main = !->
    result: I64 = double add 2, 3
    result.println!
```

The postfix `!` calls a function or method with no explicit arguments. A
method uses a value receiver and a type-associated function uses `::`.

## Structs and enums

The declaration forms are:

```text
STRUCT_DECLARATION ::= "struct" TYPE_NAME TYPE_PARAMETER_LIST? FIELD_DECLARATION*
FIELD_DECLARATION ::= "<"? IDENTIFIER ":" TYPE
ENUM_DECLARATION ::= "enum" TYPE_NAME TYPE_PARAMETER_LIST? VARIANT_DECLARATION+
VARIANT_DECLARATION ::= IDENTIFIER TYPE_LIST?
```

A complete program can combine named fields, associated construction, and
variant matching:

```rock
struct Point
    < x: I64
    < y: I64

enum Shape
    Dot Point
    Empty

impl Point
    new: I64 -> I64 -> Point
    new = x, y ->
        Point
            x: x
            y: y

describe: Shape -> &Str
describe = shape ->
    match shape
        Shape::Dot point => "dot"
        Shape::Empty => "empty"

main = !->
    point: Point = Point::new 3, 4
    shape: Shape = Shape::Dot point
    describe shape .println!
```

All required fields must be initialized. An enum constructor is qualified by
its enclosing type so that the variant's payload type is unambiguous.

## Traits, implementations, and receivers

Trait and implementation grammar is:

```text
TRAIT_DECLARATION ::= "trait" TYPE_NAME TYPE_PARAMETER_LIST? TRAIT_MEMBER+
TRAIT_MEMBER ::= "type" IDENTIFIER | RECEIVER? IDENTIFIER ":" TYPE
IMPLEMENTATION ::= "impl" TYPE_NAME TYPE_ARGUMENT_LIST? ("for" TYPE)? IMPLEMENTATION_MEMBER+
RECEIVER ::= "@" | "^@" | "~@"
```

The receiver markers express the ownership contract. This complete example
uses all three receiver modes without referring to a declaration outside the
fence:

```rock
struct Counter
    < value: I64

impl Counter
    @read: I64
    @read = -> @value

    ^@add: I64 -> ()
    ^@add = amount ->
        self.value = self.value + amount
        return

    ~@finish: I64
    ~@finish = -> self.value

main = !->
    mut counter: Counter = Counter
        value: 4
    counter.add 3
    current: I64 = counter.read!
    total: I64 = counter.finish!
    current.println!
    total.println!
```

`@read` observes, `^@add` mutates, and `~@finish` consumes. A consuming call
ends the owner's usable lifetime, so the example does not use `counter` after
`finish!`.

Bounds follow declarations and signatures:

```text
WHERE_CLAUSE ::= "where" BOUND ("," BOUND)*
BOUND ::= TYPE_NAME ":" TRAIT_PATH_LIST | TYPE_NAME IDENTIFIER ":" TRAIT_PATH_LIST
```

The following program declares a generic trait with an associated type and
implements it for a concrete type:

```rock
trait Combiner T
    type Output
    @combine: T -> Self::Output

struct AddOne

impl Combiner I64 for AddOne
    type Output = I64
    @combine = value -> value + 1

main = !->
    ()
```

`Combiner T` declares the argument type as a trait parameter, while each
implementation chooses `Output`. `AddOne` uses `I64` for both and returns its
argument plus one. This `main` returns unit without calling `combine`; see
[Generics](../language/generics.md#trait-bounds) for functions with `where` bounds.

## Type forms

Type expressions use these productions:

```text
TYPE ::= TYPE_NAME | TYPE_APPLICATION | TYPE_LAMBDA | TYPE_PROJECTION | TYPE_REFERENCE | TYPE_POINTER | TYPE_ARRAY | TYPE_SLICE | TYPE_TUPLE | TYPE_FUNCTION | "()" | "_"
TYPE_APPLICATION ::= TYPE_NAME TYPE_ARGUMENT ("," TYPE_ARGUMENT)*
TYPE_ARGUMENT ::= TYPE | "_"
TYPE_LAMBDA ::= "\\" TYPE_NAME "->" TYPE
TYPE_PROJECTION ::= TYPE_NAME "::" TYPE_NAME
TYPE_REFERENCE ::= "&" TYPE | "&mut" TYPE
TYPE_POINTER ::= "*" TYPE
TYPE_ARRAY ::= "[" TYPE ";" INTEGER_LITERAL "]"
TYPE_SLICE ::= "[" TYPE "]"
TYPE_TUPLE ::= "(" TYPE "," TYPE_LIST? ")"
TYPE_FUNCTION ::= TYPE "->" TYPE
```

The following program combines a generic struct, a generic function signature,
a concrete type application, and a shared reference:

```rock
struct Pair A, B
    < first: A
    < second: B

identity: T -> T
identity = value -> value

borrow_first: &Pair I64, &Str -> &Str
borrow_first = pair -> pair.second

main = !->
    pair: Pair I64, &Str = Pair
        first: 7
        second: "seven"
    text: &Str = borrow_first &pair
    text.println!
```

`Pair I64, &Str` fixes the two field types. Despite its name, `borrow_first`
reads `pair.second` through a shared reference, so the program prints `seven`.
The generic `identity` function is declared but not called.

Other forms in the grammar include `_` for an inference hole, `Self::Output`
for an associated type projection, `*T` for a raw pointer, and `&mut T` for a
mutable borrowed view.

## Control flow and patterns

The core control-flow productions are:

```text
CONTROL_FLOW ::= IF_EXPRESSION | WHILE_LOOP | FOR_LOOP | LOOP_EXPRESSION | MATCH_EXPRESSION
WHILE_LOOP ::= "while" EXPRESSION BLOCK
FOR_LOOP ::= "for" PATTERN "in" EXPRESSION BLOCK
LOOP_EXPRESSION ::= "loop" BLOCK
MATCH_EXPRESSION ::= "match" EXPRESSION MATCH_ARM+
MATCH_ARM ::= PATTERN ("if" EXPRESSION)? "=>" EXPRESSION
```

This complete program uses a `while`, a `for`, `break`, and `continue`:

```rock
main = !->
    mut number: I64 = 0
    while number < 3
        number = number + 1
        if number == 2
            continue
        number.println!

    for item in 3..6
        item.println!

    loop
        break
```

Patterns include bindings, literals, tuple shapes, constructor payloads, and
wildcards:

```text
PATTERN ::= "_" | IDENTIFIER | "mut" IDENTIFIER | LITERAL | TUPLE_PATTERN | ARRAY_PATTERN | INSTANCE_PATTERN | "&" PATTERN | "&mut" PATTERN | IDENTIFIER "@" PATTERN
TUPLE_PATTERN ::= "(" PATTERN "," PATTERN_LIST ")"
ARRAY_PATTERN ::= "[" ARRAY_PATTERN_ELEMENT_LIST? "]"
ARRAY_PATTERN_ELEMENT ::= PATTERN | ".." IDENTIFIER
INSTANCE_PATTERN ::= TYPE_PATH | TYPE_PATH PATTERN_LIST | TYPE_PATH FIELD_PATTERN_LIST
FIELD_PATTERN ::= IDENTIFIER ":" PATTERN
```

Here is a complete constructor-pattern example:

```rock
enum Message
    Quit
    Number I64

label: Message -> &Str
label = message ->
    match message
        Message::Quit => "quit"
        Message::Number number if number > 0 => "positive"
        Message::Number _ => "nonpositive"

main = !->
    first: &Str = label Message::Quit
    second: &Str = label Message::Number 4
    first.println!
    second.println!
```

An arm guard runs only after its structural pattern matches. The current
compiler does not diagnose every incomplete enum match, so write every variant
or finish with `_` deliberately.

## Modules, imports, and visibility

The module grammar is:

```text
MODULE ::= "mod" IDENTIFIER
IMPORT ::= ">" PATH
EXPORT ::= "<" (DECLARATION | PATH)
PATH ::= IDENTIFIER ("::" IDENTIFIER)* ("::*")?
```

`mod name` loads `name.rk` relative to the declaring file. `>` brings an item
into scope and `<` exposes an item to consumers. The following project listing
is a complete two-file source arrangement; each file is shown in its entirety
and the imported function is declared before its use in the consumer file.

```text
/tmp/rock-syntax-project/math.rk
< square: I64 -> I64
< square = value -> value * value

/tmp/rock-syntax-project/main.rk
mod math
> math::square

main = !->
    result: I64 = square 6
    result.println!
```

Inline module syntax is parsed by some tools but is not a supported compiled
project form; use this file-backed arrangement.

## Operators and postfix forms

Operators are declarations in the current program or an explicit dependency:

```text
OPERATOR_DECLARATION ::= "infix" INTEGER_LITERAL OPERATOR
INFIX_FUNCTION ::= OPERATOR "=" PARAMETER_LIST "->" BODY
PREFIX_EXPRESSION ::= OPERATOR EXPRESSION
POSTFIX_EXPRESSION ::= EXPRESSION "?" | EXPRESSION "!"
SPACED_DOT_EXPRESSION ::= EXPRESSION " ." IDENTIFIER ARGUMENTS?
```

Function application has precedence `8`. An operator declared above `8` is
part of the final argument; an operator at or below `8` applies to the call
result. A spaced dot has lower precedence and applies to the complete
expression on its left:

```rock
double: I64 -> I64
double = value -> value * 2

main = !->
    value: I64 = 3
    double value + 1 .println!
    Option::Some value <&> double .unwrap_or 0 .println!
```

Use parentheses when an operator above application precedence must instead
operate on the call result: `(double value) + 1`.

An adjacent `&` begins a shared-borrow argument: `read &value` parses as
`read (&value)`, unlike a spaced infix `&`. Postfix `?` after the final
argument propagates the whole call, including when that argument is borrowed:
`read &value?` means `(read (&value))?`. The
[error-handling chapter](../functional/error-handling.md#borrowed-call-arguments)
contains a complete example. `?` is propagation syntax, not an infix operator
whose precedence is declared by the program.

This complete program defines a pipeline operator, converts its result with
an associated string constructor, and prints it with a zero-argument method call:

```rock
infix 1 |>
|> = value, function -> function value

double: I64 -> I64
double = value -> value * 2

main = !->
    value: I64 = 7 |> double
    text: String = String::from_i64 value
    text.println!
```

`7 |> double` produces `14`, `String::from_i64 value` creates the text `14`,
and `text.println!` prints it. The standard library supplies common arithmetic
and comparison meanings, but the compiler does not assign a meaning to an
operator spelling by itself.

## Macros

Declarative macro grammar uses fragment captures:

```text
MACRO_DECLARATION ::= "macro" IDENTIFIER MACRO_ARM+
MACRO_ARM ::= MACRO_PATTERN "=>" MACRO_TEMPLATE
MACRO_INVOCATION ::= "%" IDENTIFIER MACRO_ARGUMENT*
MACRO_CAPTURE ::= "$" IDENTIFIER ":" FRAGMENT_KIND
FRAGMENT_KIND ::= "ident" | "expr" | "ty"
```

This complete macro program captures an identifier and emits a function before
`main` calls it:

```rock
macro make_value
    $name:ident =>
        $name = -> 42

%make_value answer

main = !->
    value: I64 = answer!
    value.println!
```

Macro facilities are experimental. Keep macro definitions small, and test the
expanded program through the normal compiler rather than treating expansion
text as a stable interchange format.

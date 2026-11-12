# Rock Language Reference

This reference describes the development compiler, not a stable language
standard. The [book](https://rock-lang-org.github.io/Rock/) supplies detailed
explanations and complete examples. The [Unreleased changelog](../CHANGELOG.md)
records differences from v0.7.0. Compiler regression tests remain the authority
for edge cases; parsed syntax is not necessarily supported end to end.

## Layout, Bindings, and Calls

Indentation delimits blocks. `//` introduces a line comment and `/* ... */` a
block comment. Bindings use `name = expression`; annotations use `name: Type`.
Locals shadow top-level names rather than reassigning those items. `mut`
permits mutable access; ordinary local reassignment preserves the binding's
type. Unknown annotation types are errors, while local `_` annotation holes
infer their expected kind from context.

Function arguments are comma-separated. `function!` invokes a zero-argument
function. `->` returns the body's result; `!->` discards it and returns unit;
`~>` enables currying. A unit-returning `main` exits successfully. Function
signatures are optional when inference supplies enough information.

```rock
add = left, right ~> left + right

main = !->
    increment = add 1
    increment 41 .println!
```

This prints `42`. See [Functions](src/language/functions.md),
[Bindings](src/language/bindings.md), and
[Functions as Values](src/functional/function-values.md) for call holes,
operator sections, callable bounds, and capture ownership.

## Types and Generics

Primitive spellings include `I8` through `I64`, `U8` through `U64`, `F32`,
`F64`, `Bool`, and `Char`. `Char` is a Unicode scalar. String literals have
type `&Str`; `String` owns UTF-8 storage. String lengths count bytes, slicing
checks UTF-8 boundaries, and `chars!` traverses scalars.

Composite types include `[T; N]` arrays, borrowed `&[T]` and `&mut [T]` slices,
tuples `(A, B)`, unit `()`, references `&T` and `&mut T`, raw pointers `*T`, and
function types such as `I64 -> I64`. Bare slices are not ordinary owned values.
Nested arrays support chained indexing.

Generic arguments follow the constructor with spaces and commas: `Pair I64,
Bool`. Defaults are declared in braces, for example `struct Pair A, {B = A}`.
Array lengths can be declared as `N: usize`; function signatures and impl
array shapes can introduce inferred length parameters. Constant lengths must
be literals or in-scope length parameters, not arbitrary runtime expressions.

```rock
struct Buffer T, N: usize
    < data: [T; N]

filled: [I64; N] -> [I64; N]
filled = template -> [7; N]

main = !->
    buffer: Buffer I64, 2 = Buffer
        data: filled [1, 2]
    buffer.data[1].println!
```

This prints `7`. Repeat initializers must be copyable. See
[Generics](src/language/generics.md) for defaults, aliases, bounds, associated
types, and const parameters. [Higher-Kinded Types](src/functional/higher-kinded-types.md)
covers constructor kinds, sections such as `Result _, E`, and type lambdas.

## Data, Traits, and Ownership

Structs declare named fields. Enums declare variants with optional positional
payloads. `<` exports declarations or fields. Exporting an enum exposes its
variants through the enum path; imports provide short constructor names in
expressions and patterns. The std prelude imports `Some`, `None`, `Ok`, and `Err`.

```rock
enum Light
    On
    Off

> Light::Off

main = !->
    light = Off
    match light
        Light::On => "on".println!
        Off => "off".println!
```

This prints `off`. Write exhaustive matches deliberately: the compiler does
not diagnose every incomplete match. See [Enums](src/language/enums-and-patterns.md)
for guards and borrowed payloads.

Traits declare capabilities and associated types; impls supply implementations.
`where T: Trait` supplies a generic bound. Receiver markers are `@` for shared
borrowing, `^@` for mutable borrowing, and `~@` for consuming ownership.
Owned non-copy values move; `Drop` performs deterministic cleanup. Owned array
loops move their elements and drop unvisited elements on early exit.

Closures infer capture ownership from liveness and invocation mode from body
effects. An owned capture need not make a closure consuming; a body that moves
out its capture does. Borrowed captures retain lifetime restrictions. Detached
thread tasks require `Owned` and `Send` captures and the appropriate `FnOnce`
contract. See [Traits](src/language/traits.md), [Ownership](src/language/ownership.md),
and [Concurrency](src/std/concurrency.md).

## Control Flow and Operators

`if` and `match` produce values. Loops include `while`, `loop`, and `for..in`,
with `break` and `continue`. Ranges use `..` or `..=`; open-ended forms are
useful for slicing, not an unbounded iterator API. `?` propagates a fallible
result. `do` notation sequences operations through ordinary `bind` resolution.
See [Control Flow](src/language/control-flow.md) and
[Error Handling](src/functional/error-handling.md).

Operator declarations and implementations belong to the program or its
explicit dependencies, not compiler-owned std fallbacks. Duplicate operator
declarations are errors, even across dependencies. Application precedence is
`8`; a spaced dot applies to the complete preceding expression. See
[Operators](src/language/operators.md) and the
[syntax reference](src/reference/syntax.md).

## Modules, Macros, and Packages

`mod name` loads a file-backed module. `>` imports a path; `<` exports or
re-exports a declaration or path. Unresolved imports and modules named after
dependencies are errors. Explicit imports take precedence over glob bindings.
See [Modules](src/programs/modules.md).

Declarative macros use `%name` invocations and `$name:ident`, `$name:expr`, and
`$name:ty` captures. Repetitions start with `$(` and close with `)*`, in both
matchers and templates. Templates repeat inline or by line according to their
position. Macros expand in loaded modules and expression position. Exported
macros require explicit imports; glob imports do not import them. Names in
templates resolve at the call site. Nested repetition remains unsupported.
The std prelude supplies `%format` and `%println`; see [Macros](src/programs/macros.md)
and [Prelude](src/std/prelude.md).

Packages declare one `[lib]`, any number of `[[bin]]` targets, or both in
`rock.toml`. `rock build` builds all targets; `rock run --bin NAME` selects a
binary. Path dependencies must provide libraries. Compiler-level dependencies
are explicit prebuilt artifacts; std is external, with automatic prelude import
only when explicitly supplied. See [Packages](src/programs/packages.md) and
[Tooling](src/reference/tooling.md).

## Safety and Implementation Limits

Raw-pointer and FFI operations require the appropriate unsafe boundary; a
parsed construct is not evidence that it is safe or implemented. See
[Unsafe](src/systems/unsafe.md), [FFI](src/systems/ffi.md), and
[Known Limitations](src/reference/limitations.md) before relying on an edge case.

# Types and Values

Rock is statically typed: the compiler determines a type for every expression before generating executable code. Local annotations are optional when inference is clear, while signatures make function boundaries explicit.

## Primitive values

The primitive families used in ordinary programs are:

| Family | Types | Typical literals |
| --- | --- | --- |
| Signed integers | `I8`, `I16`, `I32`, `I64` | `-5`, `42` |
| Unsigned integers | `U8`, `U16`, `U32`, `U64` | `0`, `255` |
| Floating point | `F32`, `F64` | `3.5`, `0.125` |
| Boolean | `Bool` | `true`, `false` |
| Character | `Char` | `'R'` |
| Borrowed string | `&Str` | `"rock"` |
| No meaningful value | `()` | `()` |

This complete program gives each of the commonly encountered literal forms an explicit type:

```rock
main = !->
    signed: I64 = -5
    unsigned: U64 = 255
    fraction: F64 = 3.5
    enabled: Bool = true
    letter: Char = 'R'
    text: &Str = "rock"
    signed.println!
    unsigned as I64 .println!
    fraction.println!
    enabled.println!
    letter.println!
    text.println!
```

Numeric literals receive an expected type from an annotation or a function signature. When no expected type is available, the compiler applies its numeric defaulting rules. Use fixed-width names in public signatures so callers do not have to guess.

Without an expected type, whole-number literals default to `I64` and decimal literals default to `F64`:

```rock
show_default_integer: I64 -> ()
show_default_integer = value !->
    value.println!

show_default_float: F64 -> ()
show_default_float = value !->
    value.println!

main = !->
    whole = 42
    decimal = 2.5
    show_default_integer whole
    show_default_float decimal
```

The unannotated bindings are already defaulted before the calls are checked. The functions make those inferred types explicit to the reader, and the program prints `42` and `2.5`. A surrounding annotation can choose another numeric type before defaulting occurs.

## Borrowed and owned strings

A string literal is borrowed string data with type `&Str`. `String` is a separate prelude type that owns a heap allocation:

```rock
main = !->
    borrowed: &Str = "hello"
    owned = String::from borrowed
    borrowed.println!
    owned.println!
```

The literal can be used while its source is available; `String::from` copies the bytes into owned storage. Current string helpers operate on bytes, so an index or offset is not automatically a Unicode scalar-value position.

## Annotations and signatures

Local annotations use a colon:

```rock
main = !->
    enabled: Bool = true
    distance: F64 = 12.5
    enabled.println!
    distance.println!
```

Function signatures use arrows. The following declaration and definition agree on two `F64` parameters and an `F64` result:

```rock
hypotenuse: F64 -> F64 -> F64
hypotenuse = a, b ->
    a * a + b * b

main = !->
    hypotenuse 3.0, 4.0 .println!
```

The arrows are read left to right as the function's parameter types followed by its return type. Calls still use spaces and commas. Despite its name, this `hypotenuse` body computes the sum of the squares without taking a square root, so the call prints `25`, not `5`.

## Compound type notation

Tuples, arrays, slices, references, pointers, generic applications, and function types use the following spellings. The example declares every local value it mentions:

```rock
enum DemoError
    Missing

read_shared: &I64 -> I64
read_shared = value ->
    *value

first_slice: &[I64] -> I64
first_slice = values ->
    values[0]

apply: (I64 -> I64) -> I64 -> I64
apply = function, value ->
    function value

main = !->
    pair: (I64, Bool) = (7, true)
    array: [I64; 4] = [1, 2, 3, 4]
    maybe: Option I64 = Option::Some 5
    result: Result I64, DemoError = Result::Ok 5
    mut number: I64 = 7
    shared: &I64 = &number
    shared_value = read_shared shared
    mutable_reference: &mut I64 = &mut number
    pointer: *I64 = mutable_reference as *I64
    unsafe *pointer = 9
    doubled = apply value -> value * 2, 4
    pair.0.println!
    array[0].println!
    shared_value.println!
    doubled.println!
```

The tuple has two elements, the fixed array has four `I64` elements, and `Option` and `Result` are generic prelude enums. `[I64]` is an unsized slice type, so a borrowed slice is written `&[I64]`. `&T` and `&mut T` are shared and mutable references. `*I64` is a raw pointer type; the dereference and assignment in the example are inside `unsafe` because the compiler cannot prove a raw pointer's validity.

Generic applications use spaces and commas rather than angle brackets. For example, `Result I64, DemoError` means `Result` with success type `I64` and error type `DemoError`.

## Casts

Use `as` for an explicit primitive or pointer conversion:

```rock
main = !->
    byte: U8 = 7
    count = byte as I64
    fraction = count as F64
    fraction.println!
```

The cast binds more tightly than binary operators, so each operand is converted before division:

```rock
average: I64 -> I64 -> F64
average = total, count ->
    total as F64 / count as F64

main = !->
    average 9, 2 .println!
```

A cast is still checked against the supported conversion rules; it is not a general escape from type checking. Pointer casts deserve the same care as raw-pointer dereferences.

## Common mistakes

- Writing `Str` without a reference; unsized string data is used as `&Str`.
- Using a result type without declaring or importing its failure type. This chapter uses the locally declared `DemoError`.
- Confusing `[I64; 4]` with `[I64]`: the first is fixed-size storage, the second is an unsized slice type.
- Assuming `as` changes ownership; it changes representation or numeric type, not the ownership contract.

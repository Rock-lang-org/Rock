# The Prelude and Common Traits

For an ordinary project, `rock` loads the selected toolchain's standard library and makes its prelude available automatically. The prelude is a curated set of types, traits, operators, and small utility functions. Specialized modules such as files, networking, threads, and process arguments remain explicit imports.

## What is available

The prelude exports the everyday vocabulary used by the examples in this chapter:

- owned types: `String`, `Option`, `Result`, `Vec`, `HashMap`, `Box`, `Arc`, `Mutex`, and `MutexGuard`;
- representation and ownership traits: `Show`, `From`, `Into`, `Clone`, `Drop`, `Deref`, `DerefMut`, and `Sized`;
- access and comparison traits: `Index`, `IndexMut`, `Eq`, `Ord`, and `Hash`;
- callable and thread-safety traits: `Fn`, `FnMut`, `FnOnce`, `Send`, and `Sync`;
- functional traits: `Bifunctor`, `Functor`, `Applicative`, `Monad`, `Foldable`, `Traversable`, and the `ForEach` traversal bridge;
- effect-only traversal: `for_each values, action`, for bounded ranges, foldable containers, borrowed vectors, and borrowed slices;
- the arithmetic, comparison, logical, bitwise, negation, and functional operator declarations provided by those modules.

The prelude does not turn a missing implementation into a compiler fallback: an operator still needs a matching library trait implementation. Files, process arguments, networking handles, and thread-spawning functions remain explicit imports because they are not universal vocabulary.

## Showing and printing

`Show` converts a value to an owned `String`, and `println!` writes that representation. Literal values have prelude implementations, so this complete program needs no imports.

```rock
main = !->
    42.println!
    true.println!
    "Rock".println!
```

The output is `42`, `true`, and `Rock`. Printing inspects a value where possible; it does not consume an owned value merely to display it.

An application type can implement `Show` by returning a `String`:

```rock
< struct Point
    < x: I64
    < y: I64

impl Show for Point
    @show: String
    @show = ->
        "Point(" + @x.show! + ", " + @y.show! + ")"

main = !->
    point = Point
        x: 3
        y: 4
    point.println!
```

The output is `Point(3, 4)`. The `<` markers export `Point` and its fields for use by other modules. In this example, `main` is in the same module and could construct `Point` even if its fields were private; outside modules would need an exported constructor or factory function to initialize private fields.

## Numeric behavior and generic bounds

Primitive arithmetic is supplied by stdlib trait implementations. A concrete function can use an operator directly, while generic code must name a bound that supplies the requested operation. Complete generic-bound examples appear in [Traits and Methods](../language/traits.md#generic-trait-bounds) and [Operators](../language/operators.md); this section keeps the arithmetic call concrete.

```rock
sum: I64 -> I64 -> I64
sum = left, right -> left + right

main = !->
    integer = sum 20, 22
    integer.println!
```

The output is `42`. Without a matching `Add` implementation and operator declaration, `+` has no fallback meaning. Equality, ordering, negation, bitwise operations, and indexing follow the same library-owned design.

## Conversion

`From` expresses a conversion from a source type into a destination type. The trait declares `from: T -> Self`: `T` is the source, and `Self` is the type receiving the implementation. Implement `From` when the conversion can produce a value directly, without reporting failure.

For example, an application can convert a distance in meters into a distance in millimeters:

```rock
struct Meters
    < value: I64

struct Millimeters
    < value: I64

impl From Meters for Millimeters
    from = meters ->
        Millimeters
            value: meters.value * 1000

main = !->
    distance = Meters
        value: 3
    converted = Millimeters::from distance
    converted.value.println!
```

The output is `3000`. In `impl From Meters for Millimeters`, `Meters` is the input type and `Millimeters` is the output type. The `<` markers make their fields public. The trait supplies the signature, so the implementation only needs the body of `from`. Calling `Millimeters::from distance` selects the destination explicitly; both local variable types are inferred. The argument is passed by value, so converting an owned, non-copyable source transfers ownership to `from`.

This example assumes the scaled distance fits in `I64`. For a conversion that needs validation or can fail, use a function returning `Result` so callers can handle the error. Implementing `From` in one direction does not define the reverse conversion.

The standard library uses the same trait to convert `&Str`, `&[U8]`, `I64`, `F64`, and `Char` values into owned strings. Each call below selects a different `From` implementation from its argument type:

```rock
main = !->
    from_text = String::from "hello"
    from_integer = String::from 42
    from_float = String::from 3.5
    from_character = String::from 'R'
    bytes = [82 as U8, 111 as U8, 99 as U8, 107 as U8]
    from_bytes = String::from (&bytes)
    from_text.println!
    from_integer.println!
    from_float.println!
    from_character.println!
    from_bytes.println!
```

The output is `hello`, `42`, `3.5`, `R`, and `Rock`. Borrowed text and bytes are copied into the new string's owned storage.

### Converting with `Into`

`Into` performs the same conversion using method syntax on the source value. The standard library provides `Into U for T` whenever `U` implements `From T`, so implementing `From` also makes `into!` available automatically. This applies to the custom distance conversion above as well as the string conversions.

Unlike `String::from value`, `value.into!` does not name its destination. The surrounding code must supply that type, either through an annotation or through a function parameter:

```rock
print_text: String -> ()
print_text = text !-> text.println!

main = !->
    text: String = 42.into!
    text.println!
    print_text 'R'.into!
```

The output is `42` and `R`. The `String` annotation on `text` selects the first conversion's destination; the `print_text` parameter supplies the destination for the second. `into!` takes its receiver by value, transferring ownership when the source is non-copyable. Prefer implementing `From` and using this automatic `Into` implementation rather than writing both yourself.

### Primitive casts

Primitive casts use `as`; they do not call a custom `From` implementation:

```rock
main = !->
    integer = 42
    floating = integer as F64
    floating.println!
```

## Memory and callable traits

`Clone` explicitly creates another owner, `Drop` supplies deterministic cleanup, and `Deref` forwards access through wrappers such as `Box` and `Arc`. `Fn`, `FnMut`, and `FnOnce` describe callable ownership; `Send` and `Sync` are checked when values cross a spawned thread. This example demonstrates clone and dereference without hiding the declarations involved.

```rock
main = !->
    original = String::from "owned"
    duplicate = original.clone!
    shared = Arc::new duplicate
    *shared .println!
    original.println!
```

The output is `owned` twice. `duplicate` moves into `Arc`, while `original` remains independent because `clone!` created a second allocation. A shared reference is cheaper than `Clone` when a second owner is not required.

## Explicit imports remain healthy

A prelude reduces noise for universal vocabulary; it should not hide domain dependencies. This complete program imports file, I/O, and thread APIs and uses each one.

```rock
> stdlib::fs::File
> stdlib::io::IoError
> stdlib::io::Write
> stdlib::thread::spawn

write_marker: &Str -> Result I64, IoError
write_marker = path ->
    mut file = File::create path?
    file.write_str "marker"

main = !->
    match write_marker "rock-prelude-marker.txt"
        Result::Ok count => count.println!
        Result::Err _ => 1
    match spawn (-> 7)
        Result::Ok handle =>
            match handle.join!
                Result::Ok value => value.println!
                Result::Err _ => 1
        Result::Err _ => 1
```

The output is `6` and `7`, and the marker file contains `marker`. The imports document the module boundary even though `Result`, `String`, and numeric operators come from the prelude. Keep specialized imports at the top of the fence or source file so a reader can see the dependency without searching another chapter.

## Current limits and common mistakes

- The prelude comes from the standard library in the selected toolchain; keep the compiler and toolchain components on the same revision.
- `no_std` packages do not receive these names automatically.
- An operator spelling is not a guarantee that every type supports it; trait selection still needs one applicable implementation.
- `Clone` may allocate and is not the same as compiler-known copying of primitive values.
- Domain modules remain explicit because a universal prelude should not conceal filesystem, network, process, or scheduling effects.

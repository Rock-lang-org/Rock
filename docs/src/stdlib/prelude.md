# The Prelude and Common Traits

For an ordinary project, `rock` loads the selected toolchain's standard library and makes its prelude available automatically. The prelude is a curated set of types, traits, operators, and small utility functions. Specialized modules such as files, networking, threads, and process arguments remain explicit imports.

## What is available

The prelude exports the everyday vocabulary used by the examples in this chapter:

- owned types: `String`, `Option`, `Result`, `Vec`, `HashMap`, `Box`, `Arc`, `Mutex`, and `MutexGuard`;
- representation and ownership traits: `Show`, `From`, `Clone`, `Drop`, `Deref`, `DerefMut`, and `Sized`;
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
    integer: I64 = sum 20, 22
    integer.println!
```

The output is `42`. Without a matching `Add` implementation and operator declaration, `+` has no fallback meaning. Equality, ordering, negation, bitwise operations, and indexing follow the same library-owned design.

## Conversion

The `From` family expresses reusable conversions, while primitive casts use `as`. The common `String` constructors are prelude-accessible.

```rock
main = !->
    from_text: String = String::from_str "hello"
    from_integer: String = String::from_i64 42
    from_float: String = String::from_f64 3.5
    from_character: String = String::from_char 'R'
    from_text.println!
    from_integer.println!
    from_float.println!
    from_character.println!
```

The output is `hello`, `42`, `3.5`, and `R`. Use an explicit constructor or trait conversion when ownership and failure behavior matter; use `as` for a primitive representation cast whose validity is already understood by the caller.

## Memory and callable traits

`Clone` explicitly creates another owner, `Drop` supplies deterministic cleanup, and `Deref` forwards access through wrappers such as `Box` and `Arc`. `Fn`, `FnMut`, and `FnOnce` describe callable ownership; `Send` and `Sync` are checked when values cross a spawned thread. This example demonstrates clone and dereference without hiding the declarations involved.

```rock
main = !->
    original: String = String::from_str "owned"
    duplicate: String = original.clone!
    shared: Arc String = Arc::new duplicate
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

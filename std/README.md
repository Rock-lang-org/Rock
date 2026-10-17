# Rock standard library

`rock.toml` declares `src/lib.rk` as the library entry point. All Rock sources
live under `src/`; each domain directory has a `mod.rk` that declares its
children and re-exports its public API.

| Module | Responsibility |
| --- | --- |
| `mem` | Allocation, owned heap values, destruction, sizes, and memory helpers |
| `cmp` | Equality and ordering |
| `num` | Arithmetic traits and numeric helpers |
| `ops` | Bitwise and unary operators, dereferencing, indexing, and ranges |
| `functional` | Callables, composition, mapping, binding, folding, and traversal |
| `collections` | Vectors, hash maps, borrowed slices, and eager traversal |
| `string` | Owned strings and borrowed UTF-8 operations |
| `control` | Option, Result, and short-circuiting protocols |
| `io` | Shared I/O contracts, files, and networking |
| `concurrency` | Shared ownership, mutexes, thread-safety markers, and threads |
| `ffi` | C library declarations |

`clone`, `convert`, `show`, `hash`, and `env` remain standalone modules.

## Integer arithmetic and comparisons

Integer types provide `checked_*`, `wrapping_*`, `saturating_*`, and
`overflowing_*` methods through `Integer`. Checked methods return `Option`;
overflowing methods return a wrapped value and an overflow flag. Explicit
methods do not depend on the build profile.

```rock
main = !->
    largest = 255 as U8
    (largest.wrapping_add (1 as U8)).println!
    (largest.saturating_add (1 as U8)).println!
    match largest.checked_add (1 as U8)
        Option::Some value => value.println!
        Option::None => "overflow".println!
```

This prints `0`, `255`, and `overflow`. Ordinary integer addition, subtraction,
multiplication, negation, and shifts check overflow in development builds and
wrap in release builds; division by zero and signed minimum divided or
remaindered by `-1` always fail. A prebuilt dependency retains the policy with
which it was compiled, so rebuild std when selecting a different policy.

`PartialEq {Rhs = Self}` provides equality with a default right-hand operand type;
same-type bounds use `T: PartialEq`, while cross-type bounds use `T: PartialEq U`. `String` and `&Str` support equality in
both directions. `PartialOrd` provides ordinary ordering comparisons. `Eq`
requires `PartialEq Self` and promises reflexive equality; `Ord` promises a
total ordering. Floats implement
only the partial traits: comparisons with NaN are unordered, `NaN == NaN` is
false, and `NaN != NaN` is true.
Operator precedence declarations stay in `src/lib.rk`. The prelude stays in
`src/prelude.rk`, preserving the existing automatically imported vocabulary.

Public facades keep application imports short:

```rock
> std::collections::Vec
> std::control::Option
> std::mem::Box
> std::string::String

main = !->
    mut values: Vec I64 = Vec::new!
    values.push 42
    first: Option I64 = Option::Some values[0]
    first.println!
    boxed = Box::new 42
    (*boxed).println!
    text = String::from "hello"
    text.println!
```

Files and networking remain explicit submodules: `std::io::fs::File` and
`std::io::net::TcpStream`. Threads are available through
`std::concurrency::thread`. Unsafe raw storage remains available through
`std::mem::raw_buffer`, but is not re-exported by the memory facade or prelude.
The atomic implementation module remains private.

The former flat module paths are not retained as aliases. Rebuild std
artifacts alongside the compiler and update explicit imports to the new paths.

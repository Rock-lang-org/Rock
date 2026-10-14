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
Operator precedence declarations stay in `src/lib.rk`. The prelude stays in
`src/prelude.rk`, preserving the existing automatically imported vocabulary.

Public facades keep application imports short:

```rock
> stdlib::collections::Vec
> stdlib::control::Option
> stdlib::mem::Box
> stdlib::string::String

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

Files and networking remain explicit submodules: `stdlib::io::fs::File` and
`stdlib::io::net::TcpStream`. Threads are available through
`stdlib::concurrency::thread`. Unsafe raw storage remains available through
`stdlib::mem::raw_buffer`, but is not re-exported by the memory facade or prelude.
The atomic implementation module remains private.

The former flat module paths are not retained as aliases. Rebuild stdlib
artifacts alongside the compiler and update explicit imports to the new paths.

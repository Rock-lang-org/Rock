# Modules and Visibility

Modules divide a crate into namespaces and source files. A module declaration tells the source loader which file belongs to a namespace; an import makes a public path available under a short name; an export controls what another module may use. This separation keeps implementation details local while making the public surface visible in the source.

Each example below is a separate project. In each project root, add this `rock.toml` alongside the listed source files:

```toml
[crate]
name = "module_demo"
version = "0.1.0"

[lib]
path = "main.rk"
```

Run `rock run` from that directory. The source listings in each section belong together; an entry that declares `mod math` cannot compile without its accompanying `math.rk`.

## File-backed modules

The smallest file-backed module has an entry file and a sibling file. The complete project is shown here so the import in `main.rk` has a visible definition.

Project tree:

```text
square-app/
|-- main.rk
`-- math.rk
```

### `main.rk`

```rock
mod math
> math::square

main = !->
    square 6 .println!
    math::square 4 .println!
```

### `math.rk`

```rock
double: I64 -> I64
double = value -> value * 2

square: I64 -> I64
square = value -> value * value

< square
```

`mod math` loads `math.rk` and gives it the qualified name `math`. The explicit import binds the exported function as `square`; the qualified call remains available when the name should be unambiguous. Running the project prints `36` and `16`, then returns exit code `0`.

The separate `< square` declaration exports an existing item. You can also export an item inline by putting `<` before its definition. For example, this alternative `math.rk` exports `square` at its definition and also exports a function named `myfn` that returns unit (`()`):

```rock
double: I64 -> I64
double = value -> value * 2

square: I64 -> I64
< square = value -> value * value

< myfn = -> ()
```

Both export styles make the item public; an inline export does not need a separate `< square` or `< myfn` declaration. Items without an export remain private: `double` is usable inside `math.rk` but is not importable from `main.rk`; attempting `> math::double` is a compile-time visibility error. Export only the names consumers should depend on; private helpers can then change without changing callers.

## Directory modules

When a module grows, the loader also accepts a directory containing `mod.rk`. The complete layout and both files are shown together.

Project tree:

```text
answer-app/
|-- main.rk
`-- tools/
    `-- mod.rk
```

### `main.rk`

```rock
mod tools
> tools::answer

main = !->
    answer!.println!
```

### `tools/mod.rk`

```rock
answer: () -> I64
answer = -> 42

< answer
```

The declaration is still `mod tools`; only the source location changes from `tools.rk` to `tools/mod.rk`. The program prints `42`. Do not create both `tools.rk` and `tools/mod.rk` for the same module: the loader treats multiple candidates as an error rather than choosing by declaration order.

## Imports and globs

An explicit import starts with `>` and names one public path. A glob import ends in `::*` and brings all public names from that module or facade into scope. This example uses a facade module and shows every file it loads.

Project tree:

```text
facade-app/
|-- main.rk
|-- api.rk
`-- arithmetic.rk
```

### `main.rk`

```rock
mod api
> api::*

main = !->
    add 2, 5 .println!
    double 9 .println!
```

### `api.rk`

```rock
mod arithmetic

< arithmetic::*
```

### `arithmetic.rk`

```rock
add: I64 -> I64 -> I64
add = left, right -> left + right

double: I64 -> I64
double = value -> value * 2

< add
< double
```

The `api` module re-exports the public names from `arithmetic`, and `main.rk` imports that facade. The output is `7` and `18`. Globs are useful for deliberately small facades; prefer explicit imports in application code so a new export does not silently add a name or create a conflict.

## Public structs and fields

Type visibility and field visibility are independent. Export a type and each field that consumers must construct or read. The following complete two-file project exports a struct, a constructor function, and a calculation while keeping the implementation path explicit.

Project tree:

```text
geometry-app/
|-- main.rk
`-- geometry.rk
```

### `main.rk`

```rock
mod geometry
> geometry::Point
> geometry::make_point
> geometry::distance_squared

main = !->
    point = make_point 3, 4
    point.x.println!
    point.y.println!
    distance_squared point .println!
```

### `geometry.rk`

```rock
< struct Point
    < x: I64
    < y: I64

make_point: I64 -> I64 -> Point
make_point = x, y ->
    Point
        x: x
        y: y

distance_squared: Point -> I64
distance_squared = point -> point.x * point.x + point.y * point.y

< make_point
< distance_squared
```

The output is `3`, `4`, and `25`. If `x` or `y` lost its leading `<`, outside code could still name `Point` but could not read or initialize that field. Keeping fields private is useful when a module wants to enforce construction through functions.

## Re-exporting and qualified names

Qualified paths avoid ambiguity when two modules export the same short name. This example keeps both `value` functions qualified and therefore needs no imports for them.

Project tree:

```text
qualified-app/
|-- main.rk
|-- left.rk
`-- right.rk
```

### `main.rk`

```rock
mod left
mod right

main = !->
    left::value! + right::value! .println!
```

### `left.rk`

```rock
value: () -> I64
value = -> 11

< value
```

### `right.rk`

```rock
value: () -> I64
value = -> 31

< value
```

The output is `42`. If both paths are imported under the same short name, name resolution reports a conflict; use a qualified path or choose a deliberate local facade name instead.

## Current limits and common mistakes

- Use file-backed modules. Inline-module syntax is parsed in parts of the front end but is not supported through the complete compilation pipeline.
- A missing module reports the searched file and directory candidates; check both `name.rk` and `name/mod.rk` layouts.
- A circular module load is an error; move shared declarations into a third module rather than creating a cycle.
- Imports do not make private declarations public. Add `<` in the defining module, then import the exported path.
- `::` selects a namespace or associated item, while `.` selects a field or method on a value.
- A module fence that relies on another file is only complete when that companion file is shown in the same subsection, as in the examples above.

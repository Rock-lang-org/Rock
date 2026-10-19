# Packages and Dependencies

A package is a buildable Rock crate described by `rock.toml`. The manifest names the crate, selects its source entry, and records dependencies. `rock` builds the package graph and materializes its artifacts.

## A minimal package

The manifest requires `[crate]` with a name and version, and at least one target. A package may declare one `[lib]` for a reusable library, any number of `[[bin]]` sections for executables, or both. The library defaults to `src/lib.rk`. A binary named after the package defaults to `src/main.rk`; other named binaries default to `src/bin/<name>.rk`. Each target can override `path`. This complete binary project keeps its source beside the manifest:

Project tree:

```text
hello-package/
|-- rock.toml
`-- main.rk
```

### `rock.toml`

```toml
[crate]
name = "hello"
version = "0.1.0"

[[bin]]
path = "main.rk"
```

### `main.rk`

```rock
main = !->
    "project entry point".println!
```

From `hello-package/`, `rock build` compiles the entry and writes products below `build/`; `rock run` builds and executes the linked program. A binary crate must define `main`. The output is `project entry point` and the program exits with status `0` because `main` returns unit.

## A path dependency

Path dependencies are the supported way to compose local packages today. The example has two complete projects and shows the whole source surface on both sides of the dependency.

Project tree:

```text
workspace/
|-- geometry/
|   |-- rock.toml
|   `-- lib.rk
`-- app/
    |-- rock.toml
    `-- main.rk
```

### `geometry/rock.toml`

```toml
[crate]
name = "geometry"
version = "0.1.0"

[lib]
path = "lib.rk"
```

### `geometry/lib.rk`

```rock
< struct Point
    < x: I64
    < y: I64

distance_squared: Point -> I64
distance_squared = point -> point.x * point.x + point.y * point.y

< distance_squared
```

### `app/rock.toml`

```toml
[crate]
name = "geometry-app"
version = "0.1.0"

[[bin]]
path = "main.rk"

[dependencies]
geometry = { path = "../geometry" }
```

### `app/main.rk`

```rock
> geometry::Point
> geometry::distance_squared

main = !->
    point = Point
        x: 3
        y: 4
    distance_squared point .println!
```

`rock build` builds `geometry` before `geometry-app`, then passes the fresh dependency artifact to the root compilation. The application prints `25`. Path dependencies must provide `[lib]`; a binary-only package cannot be imported as a dependency. A dependency's binary targets are not built for its consumers. The dependency name in the manifest is also the crate path used by `> geometry::Point` and `> geometry::distance_squared`; the imported names must have been exported by `geometry/lib.rk`.

Version-only registry dependencies are accepted by manifest parsing but are not resolved by the current package workflow. Use a path dependency for a local project and do not assume that `version = "1.0"` downloads or locates a package.

## A library and several binaries

Use one `[lib]` and repeated `[[bin]]` sections to share a library between executables in the same package. Each binary needs a unique name. This project contains three complete source files:

```text
tools/
|-- rock.toml
`-- src/
    |-- lib.rk
    |-- main.rk
    `-- bin/
        `-- check.rk
```

### `tools/rock.toml`

```toml
[crate]
name = "tools"
version = "0.1.0"

[lib]

[[bin]]
name = "tools"

[[bin]]
name = "check"
```

### `tools/src/lib.rk`

```rock
< message = -> "shared library"
```

### `tools/src/main.rk`

```rock
> tools::message

main = !->
    message!.println!
```

### `tools/src/bin/check.rk`

```rock
> tools::message

main = !->
    "checking".println!
    message!.println!
```

The binaries import `tools::message` from their own package's library without adding a self-dependency to the manifest. From `tools/`, run:

```console
$ rock build
$ rock run --bin tools
shared library
$ rock run --bin check
checking
shared library
```

`rock build` produces the library artifact and both executables, named `tools` and `check`, in the selected profile's build directory. Intermediate products are isolated even when custom entry paths have the same filename. `rock run --bin check` builds the library and selected binary, not the other binaries. With several binaries, `rock run` without `--bin` reports the available names instead of choosing one. With exactly one binary, the selection flag is optional. A library-only package still builds successfully with `rock run` and reports that there is no binary to execute.

## Crate artifacts

For a library target, `rock build` produces a reusable `.rkca` product containing the typed exported interface, generic bodies needed by consumers, operator declarations, dependency records, and object-linkage metadata, along with its object file. Libraries do not need a `main` function; only binary targets produce executables. `rock run` on a library-only package builds it, then reports that there is no binary to execute and exits successfully. `rock build` and `rock run` create or reuse these products for path dependencies automatically; application authors do not need to pass artifact paths between packages.

The package graph still remains explicit. Declare each dependency in `rock.toml`; the tool does not search neighboring source directories for undeclared packages. A missing or mismatched dependency is a build error.

## Prelude and `no_std`

Passing the `std` artifact makes its prelude available automatically. A low-level package can opt out in its manifest. The complete no-std source returns unit (`()`), so it does not rely on prelude operators or types.

Project tree:

```text
bare-package/
|-- rock.toml
`-- lib.rk
```

### `rock.toml`

```toml
[crate]
name = "bare"
version = "0.1.0"
no_std = true

[lib]
path = "lib.rk"
```

### `lib.rk`

```rock
< do_nothing = !->
    ()
```

Without the prelude, `String`, `Option`, familiar methods, and standard operator implementations are not injected. This is intentional: primitive operator meanings are supplied by explicit library declarations and implementations rather than by compiler-owned fallbacks. Add an explicit dependency and import when a low-level package needs a particular capability.

## Project commands and arguments

The package commands operate from the directory containing `rock.toml`:

```console
$ rock format
$ rock build
$ rock run -- first second
$ rock expand
```

`rock run -- first second` passes two user arguments after the separator; the program's argument API also includes `argv[0]`. There is currently no `rock test` subcommand. Use ordinary Rock programs or an external harness for application-level tests.

## Common mistakes and current limits

- Keep the manifest's crate name, imported crate path, and artifact name aligned.
- Declare `[lib]` once and use a separate `[[bin]]` section for each executable; change old executable manifests from `[lib]` to `[[bin]]`.
- Keep the target's `path` relative to the package root and ensure the file exists.
- Declare every path dependency in `rock.toml` and let `rock` build it in dependency order.
- Do not expect registry resolution, implicit source discovery, or compiler-owned std injection.
- `no_std = true` is a package-level choice; code that uses prelude types must opt into the appropriate explicit dependency instead.

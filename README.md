# Rock

[![Book](https://github.com/Rock-lang-org/Rock/actions/workflows/book.yml/badge.svg?branch=master)](https://github.com/Rock-lang-org/Rock/actions/workflows/book.yml)
[![Discord](https://img.shields.io/discord/990627124236939314.svg)](https://discord.gg/f6skPNB96J)

[Read the Book](https://rock-lang-org.github.io/Rock/) | [GitHub Releases](https://github.com/Rock-lang-org/Rock/releases) | [Examples](examples/)

**A native language with a functional style and explicit ownership.**

Rock combines type inference, pattern matching, traits, and higher-kinded types with native code generation through LLVM. Functions are ordinary values, containers share useful abstractions, and short operators let you write transformations in the order you read them. `do` notation keeps dependent, fallible operations in a flat block.

The syntax takes inspiration from [LiveScript](https://livescript.net/), [Haskell](https://www.haskell.org/), and [Rust](https://www.rust-lang.org/). You do not need to know those languages to follow this tour. It starts with small programs and introduces the functional vocabulary through examples.

Rock is experimental. Version `0.6.0` is not a stability promise: syntax, APIs, and tooling can change, and compiler bugs remain. Use it to explore, build small programs, and help shape the language.

## Contents

- [Install](#install)
- [Your First Program](#your-first-program)
- [Everyday Syntax](#everyday-syntax)
- [Functions and Pipelines](#functions-and-pipelines)
- [Data and Ownership](#data-and-ownership)
- [Traits](#traits)
- [Functional Operators](#functional-operators)
- [Higher-Kinded Types](#higher-kinded-types)
- [Modules and Macros](#modules-and-macros)
- [Beyond the Basics](#beyond-the-basics)
- [Tools and Editors](#tools-and-editors)
- [Build From Source](#build-from-source)
- [Keep Exploring](#keep-exploring)

## Install

`rockup` is Rock's rustup-style toolchain manager. It installs the matching compiler, project command, language server, and standard library together. You use `rock` for projects and `rockup` to manage installed versions.

**Binary releases do not require Rust or an LLVM installation.** LLVM 18 is linked into the compiler. The initial release target is **x86_64 Linux GNU**, with **Ubuntu 24.04 / glibc 2.39** as its baseline. A C linker and standard system libraries are still required; these are not fully static executables.

> **Release format:** rockup requires `v0.5.0` or later. Historical releases use a different asset layout and cannot be installed through this bootstrap.

### Install the Toolchain

On Ubuntu 24.04, install the small set of system prerequisites:

```sh
sudo apt install build-essential curl ca-certificates
```

GNU tar, gzip, and `sha256sum` must also be available; they are normally already installed on Ubuntu. The installer does not run `sudo` or install system packages for you.

Install Rock in one step. This executes a script from the release publisher, so only run it if you trust that publisher; you can inspect [`install.sh`](https://github.com/Rock-lang-org/Rock/releases/latest/download/install.sh) separately first.

```sh
curl --proto '=https' -fsSL https://github.com/Rock-lang-org/Rock/releases/latest/download/install.sh | sh
```

The bootstrap downloads and verifies the standalone rockup manager, then runs `rockup self install` to copy it and command shims into `~/.rockup/bin` and add shell setup. It then runs the installed manager's `rockup install stable` automatically to install the compiler, project command, language server, and standard library. No separate install command is needed.

Rockup verifies the toolchain archive before unpacking it. Checksums detect corrupted downloads; they are not independent signatures of the release publisher. An optional `vVERSION` argument to the bootstrap pins both the manager and the toolchain.

Restart your shell, or activate a POSIX-compatible shell with `. "${ROCKUP_HOME:-$HOME/.rockup}/env"`, then check:

```sh
rock --version
rockup list
```

### Keep It Updated

Once rockup is installed, the everyday commands are short:

| Command | What it does |
| --- | --- |
| `rockup install` | Install the latest stable toolchain |
| `rockup update` | Install or update the stable toolchain |
| `rockup self update` | Update the rockup manager itself |
| `rockup list` | List installed toolchains and mark the active one |
| `rockup install v0.6.0` | Install that specific release |
| `rockup default v0.6.0` | Select that version, installing it if needed |
| `rockup run v0.6.0 rock --version` | Run one command with a chosen version |
| `rockup remove v0.6.0` | Remove an installed version |

`install` and `update` default to `stable`, meaning GitHub's latest non-prerelease release. The channel name does not mean Rock's language or APIs are stable. Use `update`, rather than another `install`, when a toolchain is already present.

To pin a project, install its version first and add a `rock-toolchain.toml` beside its manifest:

```toml
[toolchain]
channel = "v0.6.0"
```

A project pin selects an installed toolchain; it does not download one automatically. See the [installation guide](https://rock-lang-org.github.io/Rock/getting-started/installation.html) for custom `ROCKUP_HOME` locations, shell setup, and troubleshooting. Windows, macOS, musl, and other CPU architectures are not release targets yet.

## Your First Program

Create a hello-world project:

```sh
rock new hello-rock
cd hello-rock
```

The command creates a `rock.toml` manifest:

```toml
[crate]
name = "hello-rock"
version = "0.1.0"

[lib]
path = "src/main.rk"
```

The manifest names the package and its entry file. The package version is your application's version, not the compiler version. The current project format uses `[lib]` for the entry even when you run a program containing `main`.

It also creates `.gitignore` to exclude `/build/` and `src/main.rk` with this program:

```haskell
main = !->
    "Hello, Rock!".println!
```

Run it:

```console
$ rock run
Hello, Rock!
```

`main = !->` defines a function with no arguments that evaluates its body for effects, discards its result, and returns unit (`()`). When `main` returns unit, the process exits with status `0`, so no final `0` is needed. Indentation forms the body, and `.println!` calls a method with no arguments. There are no statement-ending semicolons.

Prefer `main = !->` for ordinary programs. Use `main = ->` when you intentionally return an integer process exit status instead; the discard form does not turn a discarded error value into a failure status.

**Trying the tour:** each Rock code block below is a complete replacement for `src/main.rk`, independent of earlier blocks. The modules section shows its complete two-file project separately. Standard prelude names such as `Option`, `Vec`, and `println` are available through the installed stdlib; non-prelude imports are shown explicitly. In the examples, `|>` passes a result to the next function, and `(.println!)` is a function that prints its input.

## Everyday Syntax

### Values, Types, and Expressions

A binding uses `name = value`. Rock infers types from expressions and how values are used, including across function calls. These examples leave ordinary function signatures and local types to inference; declarations such as struct fields, trait contracts, and foreign APIs still state their types.

```haskell
category = temperature ->
    if temperature < 0
        "freezing"
    else if temperature < 20
        "cool"
    else
        "warm"

main = !->
    category 12 .println!
```

This prints `cool`. The comparison and call supply enough information to infer the numeric input and borrowed string result. The `if` expression produces the function's result: there is no separate `return` on each branch.

### Arrays, Tuples, Slices, and Loops

An array has a fixed length; a tuple groups values that can have different types. Index arrays with `[index]` and tuple fields with `.0`, `.1`, and so on.

```haskell
main = !->
    values = [10, 20, 30]
    label = ("total", 3)
    mut total = 0

    for index in 0..3
        total = total + values[index]

    label.0.println!
    total.println!
    &values[..2] .println!
```

This prints `total`, `60`, and `[10, 20]`. The range `0..3` visits indices `0`, `1`, and `2`, excluding its upper bound. `&values[..2]` borrows the first two elements as a slice; it does not create a new owned array. `mut` makes mutable access explicit. Rock also has `while`, `loop`, `break`, and `continue`; see [Control Flow](https://rock-lang-org.github.io/Rock/language/control-flow.html).

## Functions and Pipelines

Function arguments are comma-separated: `add 2, 3`. Parentheses group an expression; they are not required around every call.

`|>` passes a value to the function on its right, so a series of transformations reads from left to right.

```haskell
double = value -> value * 2

add_one = value -> value + 1

main = !->
    20
        |> double
        |> add_one
        |> (.println!)
```

This prints `41`. Each step receives the previous step's result, so there is no need to name an intermediate answer. `(.println!)` is a method section: a function that calls `println!` on its input. Use `|> (.println!)` to finish an existing multiline operator chain; otherwise, append `.println!` directly, with a space before the dot when it should apply to the whole call or expression. Each named function remains an ordinary function you can call directly.

### Lambdas, Currying, and Call Holes

You can name a small function, pass a lambda directly, or create a function by leaving an argument open.

```haskell
add = left, right ~> left + right

multiply = left, right -> left * right

main = !->
    5
        |> add 10
        |> multiply _, 2
        |> (.println!)

    5
        |> (* 2)
        |> (value -> value + 1)
        |> (.println!)
```

This prints `30` and `11`.

The `~>` in `add`'s definition enables currying. The ordinary `->` in `multiply` does not; a call hole explicitly leaves its first argument open instead.

- `~>` declares a curried function: `add 10` returns a function waiting for the second argument.
- `_` in a call argument is a hole: `multiply _, 2` waits for the number to multiply by `2`.
- `(* 2)` is an operator section: it means `value -> value * 2`.
- `value -> value + 1` is a lambda passed directly to the pipeline.

Use the form that is easiest to read. Short punctuation is useful when it removes repetition, not when it hides the operation.

## Data and Ownership

### Enums and Pattern Matching

An enum describes alternatives. A generic parameter lets the same definition carry different types.

```haskell
enum Answer T
    Value T
    Missing

value_or = fallback, answer ~>
    match answer
        Answer::Value value => value
        Answer::Missing => fallback

main = !->
    Answer::Value 42
        |> value_or 0
        |> (.println!)
    Answer::Missing
        |> value_or 7
        |> (.println!)
```

This prints `42` and `7`. `Answer T` is one definition; its element type is inferred from the payload and fallback. `match` checks the variant and binds its payload. Currying lets each pipeline supply the answer after choosing a fallback.

The standard library's `Option T` follows the same idea with `Some` and `None`. `Result T, E` adds a separate error type with `Ok` and `Err` variants.

### Structs and Receiver Modes

A struct groups named fields. An `impl` block adds methods. The receiver marker states whether a method borrows, mutates, or consumes the receiver.

```haskell
struct Counter
    < value: I64

impl Counter
    @read = -> self.value

    ^@increment = ->
        self.value = self.value + 1
        return

    ~@finish = -> self.value

main = !->
    mut counter = Counter
        value: 41
    counter.increment!
    counter.read!.println!
    counter.finish!.println!
```

This prints `42` twice. `< value` makes the field public. `return` without a value returns unit, written `()` in a type.

| Receiver | Meaning | Example above |
| --- | --- | --- |
| `@` | Shared borrow; the caller keeps the value | `read!` |
| `^@` | Mutable borrow; requires mutable access | `increment!` |
| `~@` | Consume the receiver | `finish!` |

After `finish!`, `counter` has moved and cannot be used again. Ownership also applies to ordinary function arguments and closure captures; it is not limited to methods.

### Borrowing and Cloning

`String` owns its allocation. Passing `&text` lends access without transferring that ownership. Use `clone!` when you really need another owner.

```haskell
length = text -> text.len!

main = !->
    text = String::from "Rock"
    length &text .println!
    text.println!
    text.clone!.println!
```

This prints `4`, then `Rock` twice. Borrowing does not copy the allocation; cloning does. Owned values are cleaned up deterministically through `Drop`. See [Ownership](https://rock-lang-org.github.io/Rock/language/ownership.html) for moves, mutable references, and lifetime constraints.

### Transforming Collections

`Vec T` is a growable owned sequence. Its methods make a useful starting point before the more general functional traits.

```haskell
main = !->
    mut numbers = Vec::new!
    for value in [1, 2, 3, 4]
        numbers.push value

    numbers.filter (value -> *value % 2 == 0)
        <&> (* 10)
        |> (.println!)
```

This prints `[20, 40]`. The filter callback borrows each element, so `*value` reads through that reference. `<&>` maps over the remaining elements by value; the [functional operators](#functional-operators) section explains it in detail. Both transformations consume their source vector and return a new vector, without needing a name for each intermediate collection.

## Traits

A trait names a capability. An implementation provides it for a type, and a generic function can require that capability without naming the concrete type.

```haskell
trait Area
    @area: I64

struct Rectangle
    < width: I64
    < height: I64

impl Area for Rectangle
    @area = -> self.width * self.height

area_of = shape -> shape.area!

main = !->
    rectangle = Rectangle
        width: 6
        height: 7
    area_of &rectangle .println!
```

This prints `42`. The trait declares the `area` capability; the implementation supplies it for `Rectangle`. The helper's types are inferred from the method call and its use. Passing `&rectangle` borrows the value, so calculating an area does not consume it.

The standard library uses the same trait mechanism for arithmetic, comparisons, cloning, display, and container operations. For example, `println!` is supplied through `Show`, rather than being a special print statement.

### Associated Types

An associated type lets each implementation choose a type that belongs to its contract.

```haskell
trait Source
    type Item
    @read: Self::Item

struct Number
    < value: I64

impl Source for Number
    type Item = I64
    @read = -> self.value

main = !->
    source = Number
        value: 42
    source.read!.println!
```

This prints `42`. `Source` does not require every implementation to return `I64`; this implementation chooses it with `type Item = I64`. More trait examples, including default methods, are in [Traits and Methods](https://rock-lang-org.github.io/Rock/language/traits.html).

## Functional Operators

These operators are ordinary library-defined operations. You can learn them one at a time; nothing requires writing an entire program as a chain of symbols.

### Map a Value Inside a Container

`<$>` applies a function inside a container. `<&>` performs the same operation with the arguments reversed.

```haskell
double = value -> value * 2

main = !->
    double
        <$> Option::Some 21
        |> (.println!)
    Option::Some 21
        <&> double
        |> (.println!)
    Option::None
        <&> double
        |> (.println!)
```

This prints `Some(42)`, `Some(42)`, and `None`. The function sees the integer, not the `Option`. When the value is absent, the function is not called.

This operation is called **mapping**, and the trait behind it is **Functor**. It transforms contents while preserving the kind of container: an option stays an option and a vector stays a vector.

### Choose a Fallback

`<|>` chooses the first present option:

```haskell
main = !->
    Option::None
        <|> Option::Some 7
        |> (.println!)
```

This prints `Some(7)`. The fallback is another option, unlike `unwrap_or`, which supplies a plain value. Both operands are evaluated before the operator runs; `<|>` is not a lazy conditional.

### Chain Steps That Can Fail

Mapping a function that itself returns `Option` would produce nested options. `>>=` chains such steps and keeps a single layer.

```haskell
half_even = value ->
    if value % 2 == 0
        Option::Some value / 2
    else
        Option::None

main = !->
    Option::Some 84
        >>= half_even
        >>= half_even
        |> (.println!)
    Option::Some 3
        >>= half_even
        |> (.println!)
```

This prints `Some(21)` and `None`. Each successful step passes its payload to the next function; `None` skips subsequent callbacks. This operation is called **binding**, and its trait is **Monad**.

### Results, Error Mapping, and `?`

The same mapping and binding ideas apply to successful `Result` values. `<!>` maps the error side instead. Inside a function, `?` is the direct way to extract a success or return the failure early.

```haskell
divide = numerator, denominator ->
    if denominator == 0
        Result::Err "division by zero"
    else
        Result::Ok numerator / denominator

quarter = value -> divide (divide value, 2?), 2

describe_error = message -> "calculation: " + String::from message

main = !->
    quarter 84 .println!
    divide 12, 0
        <!> describe_error
        |> (.println!)
```

This prints `Ok(21)` and `Err(calculation: division by zero)`. In `quarter`, the parentheses group the inner call so `?` applies to its result before the outer division. The error mapper changes the error type from a borrowed `&Str` to an owned `String`.

`<!>` is supplied through **Bifunctor**, a trait for constructors with two type parameters. `?` also works with `Option`; its general behavior is described by the `Try` and `FromResidual` traits. See [Error Handling](https://rock-lang-org.github.io/Rock/functional/error-handling.html) for propagation across function boundaries.

### Keep Fallible Work Flat with `do`

Use `do` when later operations need values from earlier ones. This program creates a file, writes two lines, and adds their byte counts without nesting callbacks:

```haskell
> stdlib::fs::File
> stdlib::io::IoError
> stdlib::io::Write

write_greeting: &Str -> Result I64, IoError
write_greeting = path -> do
    file <- File::create path
    first <- file.write_str "Hello from Rock\n"
    second <- file.write_str "Again\n"
    total = first + second
    pure total

main = !->
    write_greeting "greeting.txt" .println!
```

On success, this prints `Ok(22)` and leaves both lines in `greeting.txt`; creating the file replaces any existing contents. Each `<-` binds a successful payload and makes it available to the remaining steps. If creating or writing the file fails, subsequent steps are skipped and the error becomes the result. Earlier writes are not rolled back, and the file closes automatically when its owner is dropped.

`total = first + second` is an ordinary local binding. `pure total` wraps that value in the inferred carrier—here, `Result::Ok`. The final expression is returned unchanged, so the block can also end directly with another fallible operation.

`do` is syntax sugar for calls to the `bind` function in scope; the prelude supplies its **Monad** implementation. It also works with `Option` and custom carriers supported by `bind`, using the same closure and ownership rules as explicit callbacks. See [Flat Sequencing with `do`](https://rock-lang-org.github.io/Rock/functional/error-handling.html#flat-sequencing-with-do) for more examples.

### Apply a Wrapped Function

`<*>` applies a function that is itself inside a container to an argument inside the same kind of container. The responsible trait is **Applicative**.

```haskell
add = left, right ~> left + right

main = !->
    add
        <$> Option::Some 20
        <*> Option::Some 22
        |> (.println!)
    Option::Some (+ 20)
        <*> Option::None
        |> (.println!)
```

This prints `Some(42)` and `None`. Mapping the curried `add` over `Some 20` creates an optional function waiting for one more number. `<*>` supplies that number only if both the function and argument are present.

Unlike `>>=`, the next input here does not depend on the previous payload. Applicative combines wrapped inputs; Monad chooses the next computation from a successful value.

### Operator Reference

| Expression | Read it as | Provided by |
| --- | --- | --- |
| `value \|> function` | Pass this value to that function | A plain library function |
| `function <$> values` | Map this function over those contents | `Functor` |
| `values <&> function` | Map those contents with this function | `Functor` |
| `wrapped_function <*> wrapped_value` | Apply inside the container | `Applicative` |
| `value >>= next` | Continue with a container-returning function | `Monad` |
| `result <!> map_error` | Transform the error side | `Bifunctor` |
| `preferred <\|> backup` | Choose the first present option | `Option` |

The mapping and binding operations consume their input containers. A short operator spelling does not bypass ownership.

## Higher-Kinded Types

Ordinary generics let a function work with a type such as `I64`. **Higher-kinded types**, or **HKT**, let it work with a type constructor such as `Option` or `Vec`.

The distinction is small but useful:

| Name | What it represents |
| --- | --- |
| `I64` | A complete type |
| `Option I64` | A complete type containing an optional integer |
| `Option` | A constructor waiting for one type argument |
| `Vec` | Another constructor waiting for one type argument |
| `Result _, &Str` | A constructor with the error fixed to `&Str`, waiting for its success type |

### Map Across Containers

You do not need a separate mapping helper for each container. `<&>` is one generic library function backed by `Functor`; its implementation is selected for the constructor being used.

```haskell
double = value -> value * 2

main = !->
    mut values = Vec::new!
    for value in [1, 2, 3]
        values.push value

    Option::Some 4
        <&> double
        |> (.println!)
    values
        <&> double
        |> (.println!)
    (Result _, &Str)::Functor::fmap double, Result::Ok 5 .println!
```

This prints `Some(8)`, `[2, 4, 6]`, and `Ok(10)`.

The first mapping operates on `Option`; the second operates on `Vec`. The callback does not inspect either container's representation. In the library's trait contract, a bound such as `F _: Functor` describes this capability: `F` takes one type argument and supports mapping.

The final call directly selects `Functor::fmap` for `(Result _, &Str)`, fixing the error type that `Result::Ok 5` alone cannot determine. The underscore leaves the success type open: it is a **type-level hole**, distinct from the call-argument holes shown earlier.

### Implement the Abstraction Yourself

These traits are not reserved for stdlib types. Here is a small container implementing `Functor`:

```haskell
enum Wrap T
    Value T

impl Functor for Wrap
    fmap = mut mapper, wrapped ->
        match wrapped
            Wrap::Value value => Wrap::Value mapper.call_mut value

impl Wrap T
    ~@unwrap = ->
        match self
            Wrap::Value value => value

main = !->
    Wrap::Value 41
        <&> (+ 1)
        |> (.unwrap!)
        |> (.println!)
```

This prints `42`. `impl Functor for Wrap` implements the trait for the constructor, not just for `Wrap I64`. Once that implementation exists, the existing `<&>` operator works with it. `unwrap!` consumes this single-variant wrapper and returns its payload; there is no missing-value case to handle.

`fmap` inherits its generic contract from `Functor`. Its callback is required to implement `FnMut`, so it is bound as `mut mapper` and invoked with `call_mut`. That lets an implementation accept callbacks that update captured state, rather than limiting it to named functions.

### Fold or Traverse a Container

Two more constructor traits address common collection tasks:

- **Foldable** reduces all elements to one result.
- **Traversable** runs a container-producing operation on each element, then collects the results inside that outer container.

```haskell
make_values = ->
    mut values = Vec::new!
    for value in [1, 2, 3]
        values.push value
    values

append_digit = pair -> pair.0 * 10 + pair.1

positive = value ->
    if value > 0 then Option::Some value else Option::None

main = !->
    Vec::Foldable::foldl append_digit, 0, make_values! .println!
    Vec::Traversable::traverse positive, make_values! .println!
```

This prints `123` and `Some([1, 2, 3])`. The fold callback receives one tuple containing the accumulator and current element. Traversal turns individual `Option I64` results into one `Option (Vec I64)`; if an element produces `None`, the overall result is `None`.

Use `traverse_m` when later callbacks should be skipped after failure. Ordinary `traverse` can still visit later elements even when the final result is already going to be absent.

### Collect Values That Are Already Wrapped

`sequence` is traversal without another transformation: it turns a container of wrapped values into a wrapped container.

```haskell
main = !->
    mut values = Vec::new!
    values.push Option::Some 4
    values.push Option::Some 5
    values
        |> sequence
        |> (.println!)
```

This prints `Some([4, 5])`. Adding a `None` entry would make the result `None`.

Not every container supports every abstraction. `Option` and partially applied `Result` implement `Applicative` and `Monad`; `Vec` currently does not. Its supported HKT operations include mapping, folding, and traversal. See [Higher-Kinded Types](https://rock-lang-org.github.io/Rock/functional/higher-kinded-types.html) for the fuller contracts and current limits.

## Modules and Macros

### Split a Program Into Files

`mod` loads a module, `>` imports a public name, and `<` exports one. With the earlier `rock.toml`, this is a complete two-file project; the labels name separate files, not inline modules:

```haskell
// math.rk
square = value -> value * value
< square

// main.rk
mod math
> math::square

main = !->
    square 6 .println!
```

`rock run` prints `36`. Exported names also have qualified paths such as `math::square`. Larger programs can use directory modules and path dependencies; see [Modules](https://rock-lang-org.github.io/Rock/programs/modules.html) and [Packages](https://rock-lang-org.github.io/Rock/programs/packages.html).

### Define an Operator

Operators have library-defined meanings and declared precedence. You can introduce one using the same function syntax:

```haskell
infix 9 %%

%% = left, right -> left * 10 + right

main = !->
    4 %% 2 .println!
```

This prints `42`. `infix 9 %%` declares the operator's precedence; the `%%` function defines what it does. Prefer named functions when a new symbol would make an API harder to learn.

### Generate Repeated Declarations

Declarative macros operate on source syntax before type checking. `%name` invokes a macro; it is different from calling a function.

```haskell
macro make_constant
    $name:ident $value:expr =>
        $name = -> $value

%make_constant answer 6 * 7

main = !->
    answer!.println!
```

This prints `42`. `$name:ident` captures an identifier and `$value:expr` captures an expression. Macro invocation arguments are token patterns, not the comma-separated argument list of an ordinary function call. Use `rock expand` to inspect generated code; macro support is still experimental.

## Beyond the Basics

Functional code is not limited to numbers and options. The standard library includes strings, vectors, hash maps, files, TCP networking, threads, atomic reference counting, and mutexes.

### A Small Threaded Program

Threads use the same `Result` operators as other fallible operations. A single chain can start a task, join it, and handle either outcome:

```haskell
> stdlib::thread::spawn

main = !->
    spawn (-> 42)
        >>= (.join!)
        <&> (.println!)
        <!> _ !-> "thread failed".println!
```

On success this prints `42`. `>>=` joins the successfully created thread, `<&>` prints its successful result, and `<!>` prints a message if either creation or joining fails. The `!->` callback discards the print operation's return value and returns unit. Captured data must satisfy the thread API's ownership and `Send` requirements.

### Calling C

`extern` declares a function supplied by the native linker. Its signature must match the foreign ABI:

```haskell
extern abs: I32 -> I32

main = !->
    abs -7 .println!
```

This prints `7` using C's integer `abs`. Rock also exposes raw pointers and explicit `unsafe` operations; read the [FFI](https://rock-lang-org.github.io/Rock/systems/ffi.html) and [Unsafe Code](https://rock-lang-org.github.io/Rock/systems/unsafe.html) chapters before working with memory at that boundary.

For larger examples, explore the [TCP chat program](test_projects/new_new/main.rk), [file I/O guide](https://rock-lang-org.github.io/Rock/stdlib/io-and-files.html), and [concurrency guide](https://rock-lang-org.github.io/Rock/stdlib/concurrency.html). They combine these small building blocks rather than introducing a separate style of language.

## Tools and Editors

Run project commands from a directory containing `rock.toml`:

| Command | Purpose |
| --- | --- |
| `rock run` | Build and run the project |
| `rock run -- first second` | Pass arguments to the program |
| `rock build` | Build without running |
| `rock format` | Format the configured source file |
| `rock expand` | Inspect macro-expanded source |
| `rock artifact` | Build a reusable crate artifact |

`rockc` is the lower-level compiler used by these workflows. Application authors normally do not need to invoke it directly. There is no `rock test` command yet; application checks can be ordinary Rock programs, while compiler contributors use Cargo's test suite.

Rockup also installs `rock-lsp`. Editors can use it for diagnostics, inferred types on hover, function signatures, and signature help. Check that it is on your shell's path with:

```sh
rock-lsp --help
```

Without arguments it starts a language server over standard input/output, not an interactive prompt. Configure an LSP client to launch `rock-lsp` for `.rk` files.

### Neovim

The [rock.nvim](https://github.com/rock-lang-org/rock.nvim) plugin uses Neovim's built-in LSP client, without requiring `nvim-lspconfig`. It requires Neovim 0.11 or newer. Install it with your plugin manager, or clone that repository and add its root to `runtimepath` in `init.lua` (replace the path with your checkout):

```lua
vim.opt.runtimepath:prepend("/absolute/path/to/rock.nvim")
require("rock").setup()
```

The plugin does not install the compiler: complete rockup installation first, then open a Rock project and run `:checkhealth rock`. The [editor guide](https://rock-lang-org.github.io/Rock/getting-started/editor-and-diagnostics.html) covers local checkouts, other clients, and diagnostics. Tree-sitter highlighting is provided by [tree-sitter-rock](https://github.com/rock-lang-org/tree-sitter-rock).

### Ecosystem Repositories

Editor integrations and application libraries are maintained separately:

- [rock.nvim](https://github.com/rock-lang-org/rock.nvim): Neovim integration.
- [vscode-rock](https://github.com/rock-lang-org/vscode-rock): VS Code extension.
- [tree-sitter-rock](https://github.com/rock-lang-org/tree-sitter-rock): syntax grammar and highlighting queries.
- [rock_http](https://github.com/rock-lang-org/rock_http): HTTP library and examples.

`rock-lsp` stays in this repository with its compiler dependencies and ships with the matching toolchain.

## Build From Source

This section is for compiler contributors and users who want to build their own toolchain. Binary-release users can skip it.

Source builds need Git, Rust/Cargo, LLVM 18 development files and **static archives**, and a C toolchain. On Ubuntu 24.04:

```sh
sudo apt install git llvm-18-dev libpolly-18-dev libzstd-dev libxml2-dev zlib1g-dev libffi-dev libedit-dev libncurses-dev build-essential
export LLVM_SYS_180_PREFIX=/usr/lib/llvm-18
```

Use a checkout of the revision you want to build. For the `v0.6.0` release:

```sh
git clone --branch v0.6.0 https://github.com/Rock-lang-org/Rock.git
cd Rock
```

From the repository root, with Rust and Cargo installed, build the workspace and package the matching standard library:

```sh
cargo build --release
target/release/rockup dev stdlib package --path stdlib --sysroot target/release
target/release/rockup install dev --path target/release
target/release/rockup default dev
```

Restart your shell or use the activation command printed by rockup. The local name `dev` can coexist with downloaded, versioned toolchains. Static LLVM archives are mandatory for building the compiler; there is no fallback to shared LLVM libraries.

Compiler tests run through Cargo:

```sh
cargo test -p rock-lib
cargo test -p rockup
```

See the [release maintainer guide](docs/releases.md) for packaging `v0.6.0`, verifying assets, and creating a draft GitHub release. Publishing a release is a separate maintainer action, not a side effect of building the workspace.

## Keep Exploring

- [The Rock Programming Language](https://rock-lang-org.github.io/Rock/): the complete beginner's guide and reference.
- [Functions as Values](https://rock-lang-org.github.io/Rock/functional/function-values.html): closures, callable traits, and partial application.
- [Higher-Kinded Types](https://rock-lang-org.github.io/Rock/functional/higher-kinded-types.html): generic constructor operations in more depth.
- [Current Limitations](https://rock-lang-org.github.io/Rock/reference/limitations.html): important prototype constraints and known gaps.
- [Examples](examples/) and [test projects](test_projects/): larger programs and language experiments.
- [Contributing documentation](docs/checks/README.md): book build and syntax-highlighting checks.

Questions, small reproductions, and contributions are welcome through [GitHub issues](https://github.com/Rock-lang-org/Rock/issues) and [Discord](https://discord.gg/f6skPNB96J).

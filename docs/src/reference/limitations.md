# Current Limitations

Rock is an active prototype. The constraints below describe behavior that a
programmer can encounter today, not promises about a final language design.

## Platform and distribution

The supported code-generation target is currently
`x86_64-unknown-linux-gnu`. Binary releases target Ubuntu 24.04 (glibc 2.39
or newer) and statically link LLVM 18; end users do not need to install LLVM.
They are not fully static executables: system libraries and a C linker remain
required. On Ubuntu 24.04, install `build-essential`, `curl`, and
`ca-certificates`; GNU tar, gzip, and `sha256sum` must also be available.
The FFI-backed standard library is POSIX- and Linux-oriented. Rockup supports
binary releases starting with `v0.5.0`; there is no public package registry
workflow.

To prepare a development toolchain from a checkout, use the
[source installation prerequisites](../getting-started/installation.md#build-from-source).
Contributors need LLVM 18 development files and static archives, with no dynamic
fallback. From this repository, use:

```console
$ export LLVM_SYS_180_PREFIX=/usr/lib/llvm-18
$ cargo build --release
$ target/release/rockup dev stdlib package \
    --path stdlib \
    --sysroot target/release
$ target/release/rockup install dev --path target/release
```

Use path dependencies rather than registry-only dependencies. A complete
two-package layout is shown as `text` because each source file is a separate
compilation unit:

```text
/tmp/rock-path-project/geometry/rock.toml
[crate]
name = "geometry"
version = "0.1.0"

[lib]
path = "lib.rk"

/tmp/rock-path-project/geometry/lib.rk
< square: I64 -> I64
< square = value -> value * value

/tmp/rock-path-project/app/rock.toml
[crate]
name = "app"
version = "0.1.0"

[lib]
path = "main.rk"

[dependencies]
geometry = { path = "../geometry" }

/tmp/rock-path-project/app/main.rk
> geometry::square

main = !->
    result: I64 = square 6
    result.println!
```

The workaround is to keep the dependency local and compile from
`/tmp/rock-path-project/app` with the project command. Version-only registry
entries are parsed but are not resolved.

## Tooling

There is no `rock test` subcommand. Use a complete executable as an
application smoke test, and use the Rust integration suite for compiler work:

```console
$ cargo test -p rock-lib --test integration test_hello_world -- --exact
$ cargo test -p rock-lib --test integration test_unsafe_operator_function_requires_unsafe -- --exact
$ cargo test -p rock-lib
```

The formatter can still change some trait implementation headers, nested
generic applications, and `&mut` expressions unexpectedly. This complete
program is a small formatter regression reproducer:

```rock
increment: &mut I64 -> ()
increment = value ->
    *value = *value + 1
    return

main = !->
    mut number: I64 = 4
    increment &mut number
    number.println!
```

Format it, inspect the resulting diff, and compile the formatted file. The
workaround is to keep the pre-format source under version control and restore
the affected declaration manually when the formatter changes its meaning.
The tree-sitter grammar, formatter, and older root documentation can also lag
the active parser; compiler diagnostics are authoritative for a build.

Inline module bodies are another parser-only surface. The following listing
uses a file-backed declaration instead, but it is incomplete without its
companion module file:

```text
main.rk
mod inline

main = !->
    ()
```

`mod inline` asks the loader for `inline.rk` or `inline/mod.rk`; `inline` is
the module name, not an inline-module keyword. Supply that file and import
any exported names the entry uses. Macro diagnostics and indentation recovery
are also incomplete. The project-aware language server and Neovim integration
provide source diagnostics; see [Editors and Diagnostics](../getting-started/editor-and-diagnostics.md)
for setup and the current analysis boundaries.

## Language surface

### Guarded non-copy payloads

The compiler currently rejects a guard that consumes a non-copy enum payload.
This is a complete diagnostic reproducer:

<!-- compile-fail: unsupported guarded non-copy enum payload binding -->
```rock
enum Message
    Text String
    Empty

has_text: Message -> Bool
has_text = message ->
    match message
        Message::Text text if text.len! > 0 => true
        Message::Text _ => false
        Message::Empty => false

main = !->
    message: Message = Message::Text String::from_str "hello"
    result: Bool = has_text message
    result.println!
```

The workaround is to match without the guard and perform the condition inside
the arm, where the payload's ownership is unambiguous:

```rock
enum Message
    Text String
    Empty

has_text: Message -> Bool
has_text = message ->
    match message
        Message::Text text => text.len! > 0
        Message::Empty => false

main = !->
    message: Message = Message::Text String::from_str "hello"
    result: Bool = has_text message
    result.println!
```

Incomplete enum matches are not diagnosed in every path, so list every
variant or end with `_` even when the checker accepts less source.

### Tail conditional effects

A side-effecting call in a helper's tail conditional can be dropped when the
caller ignores the helper's result. This complete program should not be used
to rely on the print inside `choose`:

```rock
effect: I64 -> I64
effect = value ->
    value.println!
    value

choose: Bool -> I64
choose = flag ->
    if flag
        effect 7
    else
        0

main = !->
    choose true
```

Return data from the helper and consume it in the caller instead:

```rock
effect: I64 -> I64
effect = value ->
    value.println!
    value

choose: Bool -> I64
choose = flag ->
    if flag
        effect 7
    else
        0

main = !->
    chosen: I64 = choose true
    chosen.println!
```

### Operators, generic carriers, and text

Custom infix operators and trait-defined unary `-` and `!` are supported.
For example, a user-defined type can select its own unary result type:

```rock
struct Wrapper
    < value: I64

impl Neg for Wrapper
    type Output = I64
    @- = -> @value

main = !->
    value = Wrapper
        value: 7
    -value .println!
```

`?` works with the standard `Option` and `Result` carriers and with a custom
carrier that implements matching `Try` and `FromResidual` contracts. The
complete custom `MyFlow` implementation in [Option, Result, and
`?`](../functional/error-handling.md#defining-a-custom--carrier) shows both
success and early propagation. Different source and return carriers also work
when the return carrier implements `FromResidual` for the source residual.
Missing conversions are not synthesized: implement that exact conversion or
handle the failure with an explicit `match`.

Strings, characters, and Unicode text handling are byte-oriented and escaped
literal behavior is not mature. This complete program measures encoded bytes,
not user-perceived characters:

```rock
> stdlib::eq::Eq
> stdlib::hash::Hash

main = !->
    text: String = String::from_str "cafe"
    length: I64 = text.len!
    length.println!
```

Use ASCII protocol data or process a deliberately specified byte encoding until
Unicode-aware APIs are available. There is no async/await syntax or runtime;
use an ordinary synchronous function or the prototype thread API instead.

There are no top-level global or static values and no stable C-layout
attributes. Keep state in an owned value passed to `main`'s helpers, and use
raw buffers with an explicit byte layout at a C boundary rather than assuming
that a Rock struct has a C representation.

## Standard-library surface

### Strings and `Vec`

`String` length and indexing are byte-oriented. `Vec` has eager traversal
methods such as `for_each` and `map`, but no general lazy iterator API or
`pop` method. Use `swap_remove` at the last index to remove the final element
without changing the order of the remaining elements:

```rock
main = !->
    mut values: Vec I64 = Vec::new!
    values.push 10
    values.push 20
    length: I64 = values.len!
    match values.swap_remove (length - 1)
        Option::Some last => last.println!
        Option::None => "empty".println!
```

Removing an interior element with `swap_remove` moves the last element into
its slot. If interior removal must preserve order, copy the retained values
into a new vector.

### `HashMap`

`HashMap` currently lacks removal, iteration, entry APIs, and configurable
hashing. Its intended lookup surface is `new`, `len`, `insert`, `get`, and
`contains_key`:

```rock
main = !->
    mut scores: HashMap &Str, I64 = HashMap::new!
    scores.insert "Ada", 10
    key: &Str = "Ada"
    present: Bool = scores.contains_key (&key)
    if present
        score: Option &I64 = scores.get (&key)
        match score
            Option::Some value => value.println!
            Option::None => 0.println!
    else
        "missing".println!
```

An installed toolchain must contain the standard library built from the same
revision as the compiler. If these calls fail after switching revisions,
repackage and reinstall the development toolchain before diagnosing the source.

Keys must implement both `Hash` and `Eq`. The example uses static string
literals: the shipped implementations support `&Str`, not owned `String`
keys. An owned string's dereference support does not satisfy a generic trait
bound on `String` itself. Use supported key types or define a key wrapper
with the required implementations.

The workaround for removal or traversal is to rebuild a new map from known
keys, or maintain a separate owned list of keys while the map is in use.

Regression tests cover initialized `Vec` element destruction, replacement and
growth, and `HashMap` key/value destruction, overwrite and growth. This does
not establish cleanup correctness for every program. Zero-sized `Vec` elements
and `HashMap` keys or values are still rejected on insertion; use a non-zero-sized
representation when storing such values.

### Environment, networking, and concurrency

Environment-variable lookup is not available. Pass configuration as explicit
arguments instead:

```rock
> stdlib::env::args

main = !->
    arguments: Vec String = args!
    arguments.len!.println!
```

Networking is blocking IPv4 TCP only. It has no TLS, UDP, IPv6, or timeout
abstraction. This complete program demonstrates the supported bind boundary:

```rock
> stdlib::io::IoError
> stdlib::net::Ipv4Addr
> stdlib::net::SocketAddrV4
> stdlib::net::TcpListener

bind_local = ->
    address: SocketAddrV4 = SocketAddrV4::new Ipv4Addr::localhost!, 0
    TcpListener::bind address

main = !->
    result: Result TcpListener, IoError = bind_local!
    match result
        Result::Ok _ => 1.println!
        Result::Err error => error.println!
```

Use a thread around a blocking operation when limited concurrency is enough,
and design shutdown explicitly. There are no channels, condition variables,
thread pools, or async executors. Dropping a `JoinHandle` detaches the thread;
call `join!` before dropping it when the result or completion matters.
`spawn_detached` is available for owned unit-returning tasks, but returning
from `main` does not wait for them; see the complete
[detached-task example](../stdlib/concurrency.md#detached-tasks).

Thread and atomic implementations rely on platform ABI assumptions. Treat
long-running, memory-intensive, and concurrent applications as experiments
until cleanup and scheduling gaps close.

## Working with the prototype

Keep programs and dependencies pinned to one compiler revision, compile after
formatting, and prefer tested stdlib APIs over syntax found only in old plans.
When relying on a new edge case, add a parser test and an integration test.
Keep unsafe and FFI boundaries small, and report the smallest complete source
that demonstrates a limitation.

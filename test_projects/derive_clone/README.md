# Deriving Clone

This example implements a derive macro in ordinary Rock. The `macros/` crate
exports the derive named `Clone`; `app/` attaches it to two structs and uses the
generated implementations of std's `Clone` trait.

Rock writes derive attachments as `%derive Clone` on the line before a struct
or enum declaration. The macro export and the trait are separate names in
separate namespaces, so the app imports both explicitly:

```rock
> clone_derive::Clone
> std::clone::Clone

%derive Clone
struct Message
    < name: String
    < visits: I64

main = !->
    original = Message
        name: String::from "Ada"
        visits: 1
    mut copied = original.clone!
    copied.name = String::from "Grace"
    original.name.println!
    copied.name.println!
```

## Run it

From the repository root, use the development compiler and shared prebuilt `std`
bundle, as in [the format-macro example](../proc_macro/):

```sh
cd test_projects/derive_clone/app
cargo run -p rock --release -- run --release
```

The full app also derives `Clone` for a generic `Envelope T`. Its output is:

```text
original: Ada
1
copy: Grace
2
```

The clone contains its own `Message` and owned `String`; changing the copy leaves
the original usable and unchanged.

## How the derive works

Read [`macros/src/lib.rk`](macros/src/lib.rk) alongside
[`app/src/main.rk`](app/src/main.rk):

1. `[lib] proc_macro = true` makes the provider a host-side macro crate.
   `< macro derive Clone = context, input ->` exports a derive entry point.
2. `DeriveInput::parse` from `rock_syntax` reads the declaration supplied by the
   compiler and exposes its name, type parameters and fields.
3. `captured` forwards original identifier slices through `rock_macro`. This
   preserves the caller's binding information instead of printing and reparsing
   names. `MacroBuilder::call_site_ident` explicitly binds `Clone` to the trait
   imported by the app. The method identifier `clone` also uses call-site context.
4. `ImplBuilder` creates the trait implementation. For each ordinary type
   parameter, it adds a `Clone` bound; `Envelope T` gets `where T: Clone`.
5. The method returns the original target type, constructs the same struct and
   calls `.clone!` on each field.
   `OutputBuffer` collects the field initializers, and `context.quote` splices
   their original names into the generated body.
6. `.finish!` returns only the generated implementation. The compiler retains the
   original declaration and checks the implementation like handwritten code.

For a concrete struct, the result is equivalent to this complete program:

```rock
> std::clone::Clone

struct Message
    < name: String
    < visits: I64

impl Clone for Message
    @clone: Message
    @clone = -> Message
        name: self.name.clone!
        visits: self.visits.clone!

main = !->
    message = Message
        name: String::from "Ada"
        visits: 1
    copied = message.clone!
    copied.name.println!
```

## Scope

This teaching example supports structs with named fields, empty structs, and
ordinary type parameters. It reports a macro diagnostic for enums and kinded or
higher-kinded parameters. Every field type must implement `Clone`; missing
implementations produce ordinary compiler errors. Generic bounds are deliberately
conservative: every type parameter receives a `Clone` bound.

`rock_macro` owns construction and transport; `rock_syntax` owns declaration
parsing and impl layout. The derive itself only decides which implementation to
generate. Both libraries are regular path dependencies of the macro crate, while
the app depends only on `clone_derive` and the shared `std` bundle.

The distinction between copied identifiers, call-site names and template text
is important: `context.syntax` and literal text in `context.quote` introduce
fresh template-local names. They are suitable for generated local bindings, but
an existing trait or method needs explicit binding context. The example therefore
splices the call-site trait and method names and the original target type into
its implementation rather than emitting fresh `Clone` or `Self` text.

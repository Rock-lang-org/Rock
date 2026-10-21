# A tiny format macro

`macros/` defines `%format` using the `rock_macro` SDK. `app/` uses it:

```rock
> demo_macros::format

main = !->
    name = "Ada"
    message = %format "Hello {name}!"
    message.println!
```

The macro fills `{name}` from the caller's variables and returns a `String`.
Values use std's ordinary `Show` trait. This example deliberately omits format
specifiers, positional arguments and escaped braces.

Run from the repository root with the development compiler and shared `std` bundle:

```sh
cd test_projects/proc_macro/app
cargo run -p rock --release -- run
```

Output:

```text
Hello Ada!
```

The SDK and application use the shared prebuilt `std`, not local source overrides.

The macro implementation uses the SDK's checked string input, literal generation,
`$0`-style quote splices and `?` error propagation. Only the `{...}` formatting
grammar lives in the macro itself; quoting and protocol records live in the SDK.

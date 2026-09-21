# Macros

Rock has declarative token macros that generate top-level syntax before ordinary declaration collection and type checking. Macro expansion is useful when the repeated source shape cannot be expressed as a function, trait, or generic. It also makes ownership and generated control flow less obvious, so a small conventional expansion is easier to review than a clever one.

> **Current status:** Declarative macro parsing, direct captures, expression captures, and flat repetition are covered by compiler tests. Macro use remains experimental: nested repetition, indentation-sensitive arguments, diagnostics, and formatter support are incomplete. The examples below use parser- and expansion-tested syntax; inspect generated source with `rock expand` before depending on it in a package.

## A zero-argument declaration macro

The simplest matcher has no captures. The invocation is a top-level `%name` line, and the body emits an ordinary Rock declaration.

```rock
macro make_main
    =>
        main = !->
            42.println!

%make_main
```

The expansion produces a normal unit-returning `main` function, which prints `42` and exits with status `0`. The macro itself is not a runtime function and has no runtime ownership effect; only the generated declarations participate in type checking and cleanup. Keep generated names unique because two expansions that emit the same top-level name create the same conflict as handwritten declarations.

## Capturing identifiers and expressions

The tested fragment categories are `ident`, `expr`, and `ty`. The executable example below uses identifier and expression captures; the type capture is isolated in the explicitly experimental example that follows.

The invocation syntax is token-based and uses spaces rather than a function-call comma list. In the example below, `$name:ident` captures `answer` and `$value:expr` captures the expression `6 * 7`:

```rock
macro make_constant
    $name:ident $value:expr =>
        $name = -> $value

%make_constant answer 6 * 7

main = !->
    answer! .println!
```

The generated `answer` returns `42`, so this executable example prints `42`. A macro invocation always starts with `%`, while an ordinary declaration would not be expanded.

## Capturing a type

The parser recognizes `ty` captures, but end-to-end expansion of a generated struct field currently stops with a `Nothing expected this token` diagnostic. This parser-only example is explicitly experimental rather than executable; it contains no placeholder body or undefined runtime value.

```rock
macro make_wrapper
    $name:ident $inner:ty =>
        struct $name
            value: $inner

%make_wrapper Number I64
```

Here `$name:ident` captures `Number` and `$inner:ty` captures `I64`. The compiler currently reports the diagnostic during expansion. Until generated field indentation is stable, prefer handwritten structs and use macros for tested top-level function declarations.

## Repetition

A repetition matcher uses `$(` and `)*`. The following syntax is covered by the expansion tests and is deliberately flat.

```rock
macro define_values
    $( $name:ident )* =>
        $( $name = -> 9 )

%define_values first second third

main = !->
    first! .println!
    second! .println!
    third! .println!
```

The matcher captures three identifiers and the template emits three functions. The output is `9`, `9`, and `9`. Repetition consumes the captured tokens into generated declarations; it does not clone a runtime value and does not create a runtime loop. Captures in one repeated group must have the same number of elements.

The compiler also tests a direct capture used inside a repeated template. This form is useful when several generated functions should return one already-declared value:

```rock
macro define_aliases
    $source:ident $( $name:ident )* =>
        $( $name = -> $source! )

source = -> 7
%define_aliases source left right

main = !->
    left! .println!
    right! .println!
```

The output is `7` and `7`. The source declaration appears before the invocation, so the generated functions have a real target and no placeholder value. Do not nest another repetition inside either matcher or template; nested repetition is a known unsupported area even though the token grammar accepts repetition groups.

## Invocation matching and errors

Macro arms are tried in declaration order. A matcher that does not consume the invocation's tokens produces a compile-time diagnostic; it does not become a runtime branch. This example has two complete arms and therefore demonstrates a deliberate fallback without relying on an undefined error type.

```rock
macro make_value
    $name:ident $value:expr =>
        $name = -> $value
    $name:ident =>
        $name = -> 0

%make_value explicit 8
%make_value fallback

main = !->
    explicit! .println!
    fallback! .println!
```

The output is `8` and `0`. Put the more specific arm first; a broad arm can consume input before a later arm gets a chance. Invalid input is reported near the invocation, but expansion-origin labels and indentation diagnostics are still being improved.

## Expansion workflow and design rules

Use `rock expand` to inspect generated source before debugging type or ownership errors. A generated `~@` method still consumes its receiver, and a generated mutable method still requires a mutable binding; macros do not weaken the borrow checker. Prefer ordinary functions for behavior, generics for type reuse, and a macro only when the repeated declaration shape itself is the useful abstraction.

Rock does not yet expose a stable procedural-macro, derive, or attribute API for application authors. This chapter therefore documents only declarative `%name` macros.

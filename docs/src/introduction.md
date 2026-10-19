# Introduction

Rock is a full-fledged, expression-oriented programming language designed to make powerful programs easy to read. Like Rust, it embraces a rich syntax while keeping structure visible and understandable. It provides the foundations expected of a modern systems language, including type checking, ownership, and contracts for method receivers. Indentation forms blocks, function calls use spaces and commas, and operators are supplied by the program and its explicit dependencies.

This is a complete program. It uses only names supplied by the standard-library prelude and can be read from top to bottom.

```rock
enum Greeting
    Hello &Str
    Goodbye

message = greeting ->
    match greeting
        Greeting::Hello name => "Hello, " + name
        Greeting::Goodbye => "See you soon"

main = !->
    Greeting::Hello "Rock"
        |> message
        |> (.println!)
```

The program has three top-level declarations:

1. `Greeting` is an enum. A value is either `Hello` with a borrowed string payload or `Goodbye` with no payload.
2. `message` is a function. Its parameter is `greeting`, and its body is the indented expression after `->`.
3. `main` is the executable entry point. Its `!->` body runs for effects and returns unit (`()`).

The execution order is explicit. `main` constructs a `Greeting::Hello`, passes it to `message`, and prints the returned string. Its `!->` body discards the trailing value and returns unit (`()`), which the runtime maps to process exit status `0`. The `match` expression chooses the first arm whose pattern fits the value. Because the `Hello` arm binds `name`, the body can concatenate that payload with another string.

## The visual grammar

Rock uses indentation instead of braces. Every line indented farther than its header belongs to that header. The `else` keyword aligns with the `if` that owns it, and match arms align with one another.

Function declarations use `name = parameters -> body`. Calls put arguments after the callee and separate multiple arguments with commas:

```rock
add = left, right -> left + right
main = !-> add 20, 22 .println!
```

The call `add 20, 22` has two arguments. Each argument is a complete expression, so `add 2 + 3, 4` passes `5` and `4` without extra grouping. Parentheses remain useful when they change precedence or delimit a nested call containing commas; they do not replace the space-and-comma call syntax. A trailing `!` invokes a zero-argument function or method, so `println!` means “call `println` with no explicit arguments.”

Blocks are expressions. A block evaluates each earlier expression for its effects and gives the final expression as its value:

```rock
square = number ->
    number * number

main = !->
    square 6 .println!
```

The function `square` returns `36` because its final expression is `number * number`. In `main`, `.println!` runs for its effect, and `!->` discards its value and returns unit.

## Types without ceremony

Local types are commonly inferred. An annotation is useful when it documents an interface or removes ambiguity:

```rock
double: I64 -> I64
double = value ->
    value * 2

main = !->
    answer: I64 = double 21
    answer.println!
```

`double` accepts an `I64` and returns an `I64`. The annotation does not change the call syntax or create a runtime conversion. It gives the compiler a contract to check.

## What to expect from the book

The getting-started chapters build a runnable program before the language reference separates the ideas. Read [Hello, Rock!](getting-started/hello-rock.md) first, then [A First Project](getting-started/first-project.md). The language chapters revisit values, functions, control flow, and data types before introducing ownership, borrowing, traits, and reusable library interfaces.

Most examples use the standard library shipped with the selected Rock toolchain. The `rock` project command resolves that matching library automatically unless a manifest explicitly sets `no_std = true`.

### A Learning Path

Read the book in order for a first pass, or choose a checkpoint that matches your experience:

| Stage | Build or explain | What you practice |
| --- | --- | --- |
| Getting started | [FizzBuzz](getting-started/first-project.md) | Calls, branches, enum values, and loops |
| Feedback | [Editors and Diagnostics](getting-started/editor-and-diagnostics.md) | A project-aware editor and a build that verifies saved code |
| Ownership | [Arrays and slices](language/arrays-slices-tuples.md), then [borrowing](language/references.md) | Choosing an owner, borrowing a view, and ending conflicting access |
| Abstraction | [Traits](language/traits.md) and [error handling](functional/error-handling.md) | Reusable contracts and explicit failure paths |
| Byte-oriented programs | [Files](std/io-and-files.md) and [TCP](std/networking.md) | Partial reads, complete writes, and resource cleanup |
| Capstone | [An HTTP server](std/http.md) | Package dependencies, routing, ownership, protocol limits, and concurrent workers |

Higher-kinded types and unsafe programming are deeper topics, not prerequisites for writing your first application. Return to them when a reusable abstraction or a low-level boundary makes their motivation concrete.

At each stage, run the complete program first. Then change one input, predict the result, and compare it with the output or diagnostic. For the HTTP project, the chapter includes requests that check success, method rejection, and missing routes rather than relying only on a server-started message.

The examples in this book are intentionally complete. A `rock` fence either contains a whole program or a complete top-level module fragment whose declarations are all present in that fence. A name supplied by the std prelude may be used without an import; any other external name must be imported in the same file.

## What this book does not cover

This is a user guide, not a compiler implementation guide. It does not document parser data structures, intermediate representations, artifact internals, compiler-recognized item markers, or code-generation machinery. Low-level syntax appears only when it is part of a public application or library interface.

Rock is still a prototype. The broad language model is exercised by parser and compiler tests, but some ecosystem commands and library conveniences are incomplete. Each chapter calls out current behavior where it differs from the ideal a future release may provide.

## Common first mistakes

- Treating parentheses as mandatory call syntax. Write `max 5, 7`, not `max(5, 7)`.
- Forgetting that a block's last expression is its value.
- Using an enum payload without first matching the enum variant.
- Assuming a familiar operator has a compiler-defined meaning. The active program and its dependencies must provide the operator implementation.
- Running `rock` outside a project directory containing `rock.toml`.

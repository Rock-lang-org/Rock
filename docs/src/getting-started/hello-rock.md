# Hello, Rock!

Create your first project with `rock new`:

```sh
rock new hello
cd hello
```

This creates a new directory containing `rock.toml`, `src/main.rk`, and a `.gitignore` that excludes build output. Existing directories are never overwritten. Project names start with an ASCII letter and can contain letters, digits, hyphens, and underscores; `stdlib` is reserved.

The generated `rock.toml` manifest is:

```toml
[crate]
name = "hello"
version = "0.1.0"

[lib]
path = "src/main.rk"
```

The generated `src/main.rk` contains this complete program:

```rock
main = !->
    "Hello, Rock!".println!
```

Run it from the project directory:

```console
$ rock run
Hello, Rock!
```

The output contains one line, and the process exits with status zero.

## Reading the program

`main = !->` declares a function named `main` with no parameters. The arrow separates its parameter list from its body. The extra indentation makes the next line part of that body.

The string literal is borrowed string data. `.println!` calls the prelude's printing method with no explicit arguments. The `!->` body evaluates that call, discards its result, and returns unit (`()`). The runtime maps a unit-returning `main` to process exit status `0`, so no final `0` expression is needed.

Use `main = !->` for ordinary programs. Keep `main = ->` when you intentionally return an integer exit status, such as `1` to report failure; that arrow returns the body's trailing value instead of discarding it.

Calls with arguments use spaces and commas:

```rock
main = !->
    max 10, 20 .println!
```

`max` is a prelude function. The first argument is `10`, the second is `20`, and the result is bound to `maximum` before it is printed.

## Expressions and statements

A one-expression function can stay on one line or use an indented body:

```rock
square = number ->
    number * number

main = !->
    square 5 .println!
```

The final expression of `square` is its return value. Earlier expressions can perform effects before the final value:

```rock
announce_square = number ->
    "squaring a number".println!
    number * number

main = !->
    announce_square 5 .println!
```

An explicit `return` exits before the end of the block:

```rock
absolute = number ->
    if number >= 0
        return number
    0 - number

main = !->
    absolute 0 - 5 .println!
```

For `-5`, the condition is false, so execution reaches `0 - number` and produces `5`. For a nonnegative value, `return number` skips the remaining expression.

## Comments

Rock supports line and block comments:

```rock
main = !->
    // This line explains why the next value is printed.
    /* A block comment can
       cover several source lines. */
    "comments do not produce values".println!
```

Comments are ignored by the compiler. Use them for intent, constraints, or a non-obvious reason, not as a translation of every line.

## Common mistakes

- Using `!->` when you intend to return a custom integer exit status; use `->` instead.
- Writing `println("text")`; the current call form is `"text".println!`.
- Indenting a statement at the same level as `main = !->`; that makes it a separate top-level item instead of part of `main`.
- Running `rock` outside the directory containing `rock.toml`.

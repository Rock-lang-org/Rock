# Functions and Calls

Functions are the main unit of behavior in Rock. A declaration has a name, an equals sign, parameters, an arrow, and a body:

```rock
add = left, right ->
    left + right

main = !->
    add 2, 3 .println!
```

The parameter names are introduced before the body is evaluated. The final expression of `add` is its result.

## Signatures

A separate signature documents and checks a function's interface:

```rock
clamp: I64 -> I64 -> I64 -> I64
clamp = value, low, high ->
    if value < low
        low
    else if value > high
        high
    else
        value

main = !->
    clamp 120, 0, 100 .println!
```

The signature lists three parameter types followed by the return type. The branches of `clamp` all produce `I64`, so the definition satisfies the contract.

## Calling functions

Arguments follow the function and are separated by commas. Each argument consumes a complete expression, including a nested call with its own comma-separated arguments:

```rock
add = left, right ->
    left + right

double = value ->
    value * 2

main = !->
    double add 2, 3 .println!
```

The inner call owns the arguments `2, 3` and produces `5`; the outer call receives that complete result and produces `10`. Neither call needs grouping.

An inline callback can follow another argument: `for_each 1..=3, number !-> number.println!`. At this level, the comma separates call arguments, so the lambda has just the `number` parameter. Parenthesize a callback with multiple parameters, as in `apply (left, right -> left + right)`, to keep its parameter commas inside the lambda.

Use `!` for a zero-argument call:

```rock
main = !->
    mut values = Vec::new!
    values.push 10
    length = values.len!
    length.println!
```

`Vec::new!` calls an associated function through the type path. `values.push 10` passes one argument to a method, while `values.len!` calls a method with no explicit arguments. The `mut` binding is required because `push` uses a mutable receiver.

## Associated functions and paths

`::` selects an item through a type or module path. Associated functions do not receive an existing instance:

```rock
main = !->
    some = Option::Some 5
    text = String::from_i64 42
    some_text = match some
        Option::Some value => text
        Option::None => String::from_str "missing"
    some_text.println!
```

`Option::Some` constructs a prelude enum variant, and `String::from_i64` constructs an owned string through an associated function. A method would instead use a receiver and dot syntax, as `some_text.println!` does.

## Receiver contracts

Methods declare how they use `self`. A shared receiver uses `@`; a mutable receiver uses `^@`; a consuming receiver uses `~@`:

```rock
struct Counter
    < value: I64

impl Counter
    @read = -> @value

    ^@add = amount !->
        self.value = self.value + amount

    ~@finish = -> self.value

main = !->
    mut counter = Counter
        value: 4
    current = counter.read!
    counter.add 3
    total = counter.finish!
    current.println!
    total.println!
```

The first call borrows `counter` shared. The second needs a mutable receiver, so the binding is marked `mut`. The final call consumes `counter`; after it returns, that value is no longer available for another use. The markers are part of the method's type-level contract, not decoration.

## Return values and early exit

With an ordinary `->` body, the final expression is returned:

```rock
sign = value ->
    if value < 0
    then -1
    else 1

main = !->
    sign -8 .println!
    sign 8 .println!
```

Use `return` when a condition should leave the function immediately:

```rock
first_nonzero = left, right ->
    if left != 0
    then left
    else right

main = !->
    first_nonzero 3, 9 .println!
    first_nonzero 0, 9 .println!
```

For `(3, 9)`, the first branch returns `3` and skips `right`. For `(0, 9)`, execution reaches the final expression and returns `9`.

## Unit-returning functions

A function whose purpose is an effect can use `!->`:

```rock
greet: &Str -> ()
greet = name !->
    "Hello, " + name .println!

main: () -> ()
main = !->
    greet "Ada"
```

The trailing expression still runs, but its value is discarded and the function returns `()` automatically. An ordinary `->` body returns its trailing value instead. The signature of this `main` is `() -> ()`: it takes no arguments and returns unit. The runtime maps unit to process exit status `0`, so ordinary programs should use `main = !->` without a trailing `0`.

Keep `->` when `main` intentionally chooses an integer exit status:

```rock
> stdlib::env::args

main: () -> I64
main = ->
    if args!.len! > 1
        0
    else
        "expected at least one argument".println!
        1
```

This program exits with status `0` when given a user argument and `1` otherwise. Both integers are meaningful results here; `!->` would discard the selected status.

## Receiver sections

A parenthesized expression beginning with `.` creates a one-parameter function whose parameter is the receiver. `(.println!)` is shorthand for `value -> value.println!`: it returns the method's result.

Use `(!.println!)` when the callback should return `()` instead. It is shorthand for `value !-> value.println!`, which suits `for_each`:

```rock
main = !->
    for_each 1..=3, (!.println!)
    for_each (Option::Some 7), (!.println!)
```

This prints `1`, `2`, `3`, and `7`, each on its own line. The leading `!` discards the section body's result; the trailing `!` calls `println` with no explicit arguments.

The body can include method arguments and chains. The entire chain runs before its final result is discarded:

```rock
main = !->
    suffix = String::from_str "!"
    for_each (Option::Some (String::from_str "hello")), (!.concat suffix.clone! .println!)
```

This prints `hello!`. As with an explicit lambda, values referenced by the body are captured, and ordinary receiver and ownership rules still apply.

## Recursion

A function can call itself after its declaration:

```rock
factorial: I64 -> I64
factorial = n ->
    if n <= 1
    then 1
    else n * factorial n - 1

main = !->
    factorial 5 .println!
```

The recursive call decreases `n`, so the base case `n <= 1` is eventually reached. Rock does not promise tail-call optimization; use a loop when unbounded recursion could exhaust the stack.

## Multiline expressions

An operator may continue on a following line at one deeper indentation level:

```rock
total: I64 -> I64 -> I64 -> I64
total = base, tax, shipping ->
    base
        + tax
        + shipping

main = !->
    result = total 100, 8, 2
    result.println!
```

The continuation lines remain one expression. Binding intermediate values is often clearer for complicated calls because each type and ownership boundary gets a name.

## Common mistakes

- Omitting commas between call arguments.
- Adding parentheses as if Rock used Rust-style `function(arguments)` calls.
- Calling a `^@` method through a binding that is not marked `mut`.
- Using a consuming `~@` receiver and then trying to use the consumed value again.

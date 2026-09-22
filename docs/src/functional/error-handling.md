# `Option`, `Result`, and `?`

Rock represents expected absence and recoverable failure with ordinary enum values. There are no exceptions or implicit stack unwinding. A function that may not produce a value states its carrier in its return type.

## Optional Values

`Option T` has two constructors: `Option::Some` carries a `T`, and `Option::None` carries no value.

```rock
find_even: I64 -> Option I64
find_even = value ->
    if value % 2 == 0
    then Option::Some value
    else Option::None

main = !->
    present = find_even 6
    absent = find_even 7
    match present
        Option::Some value => value.println!
        Option::None => -1 .println!
    match absent
        Option::Some value => value.println!
        Option::None => -1 .println!
```

`present` has type `Option I64` and contains `Some 6`; `absent` has the same type and contains `None`. The output is `6` and `-1`. Matching makes both paths explicit.

## Results and Concrete Errors

`Result T, E` carries either `Result::Ok T` or `Result::Err E`. Always choose a concrete error type at a public boundary; the following function uses `I64` as its error code.

```rock
divide: I64 -> I64 -> Result I64, I64
divide = numerator, denominator ->
    if denominator == 0
    then Result::Err 1
    else Result::Ok numerator / denominator

main = !->
    successful = divide 10, 2
    failed = divide 10, 0
    match successful
        Result::Ok value => value.println!
        Result::Err error => error.println!
    match failed
        Result::Ok value => value.println!
        Result::Err error => error.println!
```

The success path is `Ok 5`, and the failure path is `Err 1`; the output is `5` and `1`. The error is data that the caller can inspect, transform, or return.

Domain errors can use a dedicated enum rather than an unstructured integer.

```rock
enum ParseError
    Empty
    Negative

impl Show for ParseError
    @show = ->
        match *self
            ParseError::Empty => String::from_str "empty input"
            ParseError::Negative => String::from_str "negative input"

parse_nonnegative: I64 -> Result I64, ParseError
parse_nonnegative = value ->
    if value < 0
    then Result::Err ParseError::Negative
    else Result::Ok value

main = !->
    result = parse_nonnegative -3
    match result
        Result::Ok value => value.println!
        Result::Err error => error.show!.println!
```

`parse_nonnegative` returns `Err ParseError::Negative`; the `Show` implementation turns that structured error into `negative input`, which is the output.

## Propagating with `?`

Postfix `?` unwraps a successful `Option` or `Result` value and returns early from the enclosing function on the failure branch.

```rock
twice_present: Option I64 -> Option I64
twice_present = value ->
    number = value?
    Option::Some number * 2

main = !->
    present = twice_present Option::Some 4
    absent = twice_present Option::None
    match present
        Option::Some value => value.println!
        Option::None => -1 .println!
    match absent
        Option::Some value => value.println!
        Option::None => -1 .println!
```

For the first call, `value?` produces `4` and the function returns `Some 8`. For the second, it returns `None` before the multiplication. The output is `8` and `-1`.

The enclosing return type must implement `FromResidual` for the propagated carrier's residual. With the standard implementations used here, propagation stays within `Option` or within a compatible `Result`.

```rock
positive: I64 -> Result I64, I64
positive = value ->
    if value < 0
    then Result::Err 1
    else Result::Ok value

double_positive: I64 -> Result I64, I64
double_positive = value ->
    number = positive value?
    Result::Ok number * 2

main = !->
    success = double_positive 4
    failure = double_positive -4
    success.unwrap_or 0 .println!
    failure.unwrap_or 0 .println!
```

The successful path returns `Ok 8`; the failing path returns `Err 1`, and both are handled with `unwrap_or 0`, so the output is `8` and `0`. Ownership follows ordinary return control flow: a value moved into a failed operation is not restored by `?`.

### Borrowed Call Arguments

A trailing `?` propagates a call's result even when its last argument is borrowed:

```rock
read: &I64 -> Result I64, I64
read = value -> Result::Ok *value

compute: () -> Result I64, I64
compute = ->
    value = 21
    number = read &value?
    Result::Ok number * 2

main = !->
    compute! .unwrap_or 0 .println!
```

`read &value?` means `(read (&value))?`: it borrows `value`, calls `read`, then propagates the returned `Result`. It does not apply `?` to the integer. Keep `&` adjacent to the borrowed operand to distinguish it from a spaced infix `&`. This program prints `42`.

## Consuming Combinators

`map` changes a successful payload while preserving the carrier. `and_then` calls a function that returns another carrier. These operations consume an owned receiver so an owned payload can move into the callback.

```rock
increment: I64 -> I64
increment = value -> value + 1

keep_even: I64 -> Option I64
keep_even = value ->
    if value % 2 == 0
    then Option::Some value
    else Option::None

main = !->
    mapped = Option::Some 4 .map increment
    chained = Option::Some 4 .and_then keep_even
    flattened = Option::Some Option::Some 9 .flatten!
    mapped.unwrap_or 0 .println!
    chained.unwrap_or 0 .println!
    flattened.unwrap_or 0 .println!
```

`mapped` is `Some 5`, `chained` is `Some 4`, and `flattened` is `Some 9`; the output is `5`, `4`, and `9`. The three source `Option` values are consumed independently and are not reused afterward.

`ok_or` converts an `Option T` into a `Result T, E` by supplying the error for the `None` case:

```rock
main = !->
    result = Option::Some 4 .ok_or 1
    result.unwrap_or 0 .println!
```

`Result` has the same shape for successful values and preserves the concrete error type. The annotations below specify the error type, which cannot be inferred from `Result::Ok` alone.

```rock
increment: I64 -> I64
increment = value -> value + 1

main = !->
    mapped: Result I64, I64 = Result::Ok 4 .map increment
    chained: Result I64, I64 = Result::Ok 4 .and_then value -> Result::Ok value * 2
    failed = Result::Err 7 .map increment
    mapped.unwrap_or 0 .println!
    chained.unwrap_or 0 .println!
    failed.unwrap_or 0 .println!
```

The output is `5`, `8`, and `0`; the error `7` is preserved in `failed` even though `unwrap_or` chooses the fallback for printing.

## Functional Operators

The prelude exports operator spellings for the same stdlib operations. The named methods above are usually clearer while learning the carriers.

```rock
increment: I64 -> I64
increment = value -> value + 1

keep_even_option: I64 -> Option I64
keep_even_option = value ->
    if value % 2 == 0
    then Option::Some value
    else Option::None

main = !->
    mapped = Option::Some 4 <&> increment
    chained = Option::Some 4 >>= keep_even_option
    fallback = Option::None <|> Option::Some 9
    converted = Option::Some 4 !> 1
    mapped.unwrap_or 0 .println!
    chained.unwrap_or 0 .println!
    fallback.unwrap_or 0 .println!
    converted.unwrap_or 0 .println!
```

`<&>` maps, `>>=` binds, `<|>` chooses a fallback, `!>` converts an `Option` to a `Result`, and `|>` passes a value to a function. The output is `5`, `4`, `9`, and `4`. These meanings are standard-library definitions, not compiler-owned special cases.

## Defining a Custom `?` Carrier

`?` is not limited to the two standard enums. A custom carrier participates by implementing `Try` and `FromResidual`. `Try::branch` separates a continuing payload from a short-circuit residual; `FromResidual::from_residual` rebuilds the enclosing carrier when propagation stops.

```rock
enum MyFlow T
    Value T
    Stop I64

enum MyResidual
    Stop I64

impl Try for MyFlow T
    type Output = T
    type Residual = MyResidual

    ~@branch = ->
        match self
            MyFlow::Value value => ControlFlow::Continue value
            MyFlow::Stop code => ControlFlow::Break MyResidual::Stop code

impl FromResidual MyResidual for MyFlow T
    from_residual = residual ->
        match residual
            MyResidual::Stop code => MyFlow::Stop code

next: Bool -> MyFlow I64
next = should_continue ->
    if should_continue
    then MyFlow::Value 41
    else MyFlow::Stop 7

compute: Bool -> MyFlow I64
compute = should_continue ->
    value = next should_continue?
    MyFlow::Value value + 1

main = !->
    match compute true
        MyFlow::Value value => value.println!
        MyFlow::Stop code => code.println!
    match compute false
        MyFlow::Value value => value.println!
        MyFlow::Stop code => code.println!
```

The successful call unwraps `Value 41`, adds one, and prints `42`. The failing call turns `Stop 7` into `ControlFlow::Break`, reconstructs `MyFlow::Stop 7`, returns early from `compute`, and prints `7`. Both implementations are ordinary public trait contracts; application code does not need compiler-only declarations.

## Current Limitations

`Option` and `Result` remain the everyday carriers because their residual conversions are already provided by the standard library. A custom source carrier and the enclosing return carrier may differ, provided the return carrier implements `FromResidual` for the source's exact residual type. The compiler selects that implementation; it does not invent a conversion between unrelated errors. Keep `Result` error parameters concrete and implement any missing conversion explicitly. Rock has no exception syntax; all failure propagation remains visible in a return type and ordinary control flow.

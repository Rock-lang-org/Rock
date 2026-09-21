# A First Project: FizzBuzz

FizzBuzz is a useful first project because it combines a value-producing function, an enum, guarded pattern matching, a range, and an inline callback without needing a large library. The rules are:

1. Visit the integers from 1 through 30.
2. Print `FizzBuzz` for a multiple of both 3 and 5.
3. Otherwise print `Fizz` for a multiple of 3.
4. Otherwise print `Buzz` for a multiple of 5.
5. Print the number for every other value.

## The complete program

Create a project manifest as `rock.toml`:

```toml
[crate]
name = "fizzbuzz"
version = "0.1.0"

[lib]
path = "main.rk"
```

Save the program as `main.rk` beside the manifest:

```rock
enum FizzBuzzValue
    Text &Str
    Number I64

fizzbuzz_value: I64 -> FizzBuzzValue
fizzbuzz_value = number ->
    match number
        n if n % 15 == 0 => FizzBuzzValue::Text "FizzBuzz"
        n if n % 3 == 0 => FizzBuzzValue::Text "Fizz"
        n if n % 5 == 0 => FizzBuzzValue::Text "Buzz"
        n => FizzBuzzValue::Number n

print_value: FizzBuzzValue -> I32
print_value = value ->
    match value
        FizzBuzzValue::Text text => text.println!
        FizzBuzzValue::Number number => number.println!

main = !->
    for_each 1..=30, number !->
        number
            |> fizzbuzz_value
            |> print_value
```

Run it from the project directory:

```console
$ rock run
```

The complete output is:

```text
1
2
Fizz
4
Buzz
Fizz
7
8
Fizz
Buzz
11
Fizz
13
14
FizzBuzz
16
17
Fizz
19
Buzz
Fizz
22
23
Fizz
Buzz
26
Fizz
28
29
FizzBuzz
```

## Start with the result type

The calculation has two possible shapes: a word or the original number. The enum gives both shapes one static type:

```rock
enum FizzBuzzValue
    Text &Str
    Number I64
```

`FizzBuzzValue` is the type. `Text` carries borrowed string data, while `Number` carries an `I64`. A constructor is qualified with the enum name:

```rock
enum FizzBuzzValue
    Text &Str
    Number I64

main = !->
    word = FizzBuzzValue::Text "Fizz"
    number = FizzBuzzValue::Number 7
    word_text = match word
        FizzBuzzValue::Text text => text
        FizzBuzzValue::Number value => "number"
    word_text.println!
    number_text = match number
        FizzBuzzValue::Text text => text
        FizzBuzzValue::Number value => "number"
    number_text.println!
```

Both local bindings have the same enum type even though their payloads differ. The matches make the payload types visible: `text` is `&Str` in its arm and `value` is `I64` in its arm.

## Separate calculation from output

The signature states the contract before the definition:

```rock
enum FizzBuzzValue
    Text &Str
    Number I64

fizzbuzz_value: I64 -> FizzBuzzValue
fizzbuzz_value = number ->
    match number
        n if n % 15 == 0 => FizzBuzzValue::Text "FizzBuzz"
        n if n % 3 == 0 => FizzBuzzValue::Text "Fizz"
        n if n % 5 == 0 => FizzBuzzValue::Text "Buzz"
        n => FizzBuzzValue::Number n
```

The signature says that one `I64` becomes one `FizzBuzzValue`. The body does not print, so the calculation can be reused by another output function or a test. Each match arm constructs the same enclosing enum type even though the selected variant differs.

Each `n` pattern binds the input integer within its own arm. An `if` guard checks an additional condition; `%` computes a remainder and `==` produces a `Bool`. Arms are checked from top to bottom, and the first successful arm supplies the result. Checking divisibility by 15 first is essential because a 3-only arm placed first would hide the combined case. The final unguarded `n` arm handles every remaining number.

## Recover the payload with `match`

The output function consumes one enum value and handles both variants:

```rock
enum FizzBuzzValue
    Text &Str
    Number I64

print_value: FizzBuzzValue -> I32
print_value = value ->
    match value
        FizzBuzzValue::Text text => text.println!
        FizzBuzzValue::Number number => number.println!
```

An arm pattern checks the variant and binds its payload at the same time. The arm body is evaluated only after its pattern succeeds. Listing both variants makes the output decision visible in one place.

## Traverse the range with `for_each`

`for_each` is a generic prelude function with two arguments: the values to visit and an action to run for each value. Here `1..=30` includes both endpoints, and `number !->` introduces the inline action:

1. `for_each` supplies the next integer as `number`, in ascending order.
2. `number |> fizzbuzz_value` passes that integer to the calculation and produces a `FizzBuzzValue`.
3. `|> print_value` passes that result to the output function.
4. The action's `!->` discards the printing result and returns unit (`()`).

The `|>` operator passes the value on its left to the function on its right. Indented continuation lines keep the pipeline together as one expression. There is no counter to update and no intermediate collection: bounded ranges are traversed directly, and `for_each` returns only after every action has completed. Both endpoints are required; an open-ended range is rejected at runtime.

The same function also works with foldable containers such as `Vec`, `Option`, and `Result`; [Higher-Kinded Types](../functional/higher-kinded-types.md#traversing-for-effects) explains how their shared `Foldable` abstraction supplies the traversal.

## A `for` version

An ordinary `for` loop can express the same traversal. An exclusive range written with `..` excludes its upper bound, so this version uses `1..31`:

```rock
enum FizzBuzzValue
    Text &Str
    Number I64

fizzbuzz_value: I64 -> FizzBuzzValue
fizzbuzz_value = number ->
    match number
        n if n % 15 == 0 => FizzBuzzValue::Text "FizzBuzz"
        n if n % 3 == 0 => FizzBuzzValue::Text "Fizz"
        n if n % 5 == 0 => FizzBuzzValue::Text "Buzz"
        n => FizzBuzzValue::Number n

print_value: FizzBuzzValue -> I32
print_value = value ->
    match value
        FizzBuzzValue::Text text => text.println!
        FizzBuzzValue::Number number => number.println!

main = !->
    for number in 1..31
        value = fizzbuzz_value number
        print_value value
```

The loop pattern `number` receives each range element. There is no explicit increment because the range iterator advances it.

## Extract a repeated idea

The divisibility test is a reusable function with two arguments:

```rock
is_divisible: I64 -> I64 -> Bool
is_divisible = number, divisor ->
    number % divisor == 0
```

The signature lists two `I64` parameters followed by a `Bool` result. The `->` definition receives both arguments in one call; it is not a curried `~>` definition. In a larger version of the project, the first match arm could read `n if is_divisible n, 15 => FizzBuzzValue::Text "FizzBuzz"` without changing the underlying calculation.

## Common mistakes

- Checking `% 3` before `% 15`, which changes the result for 15.
- Forgetting that `1..31` excludes 31 and therefore includes 30 as its final element.
- Placing the unguarded `n` arm first, which would match every number before any divisibility guard is checked.
- Returning a raw string from one match arm and an integer from another; all arms still need one enclosing type.
- Using `->` for an action that leaves the printing result as its return value; `for_each` expects a unit-returning action, so use `!->`.
- Assuming an enum payload is available outside its matching arm; the binding is introduced by the pattern.

The next chapters isolate each of these language features so that you can reason about the type and value flowing through every line.

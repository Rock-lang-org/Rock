# Operators

Rock does not assign permanent meanings to punctuation. The current source program and its explicit dependencies declare operator precedence and provide the function or trait implementation selected for the operand types. The standard library supplies familiar arithmetic, comparison, Boolean, and functional operators through the prelude.

## Standard Operators

With the standard library prelude, ordinary operators are selected from the operand types.

```rock
main = !->
    sum = 2 + 3
    same = sum == 5
    low = 2
    high = 8
    inside = sum >= low && sum <= high
    sum.println!
    same.println!
    inside.println!
```

The inferred types of `sum`, `same`, and `inside` are `I64`, `Bool`, and `Bool`; the output is `5`, `true`, and `true`. Operand and operator constraints determine these types without local annotations. If no implementation matches the operand types, compilation fails instead of applying a hidden built-in conversion.

## Declaring Precedence and Defining a Function

An `infix` declaration gives a symbol a precedence from `0` through `255`. The function with the same symbol supplies its meaning.

```rock
infix 9 %%

%%: I64 -> I64 -> I64
%% = left, right -> left - right

main = !->
    result = 40 %% 2
    result.println!
```

The declaration makes `%%` parse as an infix operator at precedence `9`; the function returns `left - right`, so the output is `38`. A dependency can transport its declarations, which is why two dependencies should not assign conflicting precedence to the same symbol.

## Trait-Selected Operators

An operator can be a trait method. This example defines a new operator and selects an implementation from the left operand's type and the right operand's type.

```rock
infix 9 %%

struct Boxed
    < value: I64

trait Combine Rhs
    @%%: Rhs -> I64

impl Combine Boxed for Boxed
    @%% = other -> @value + other.value

main = !->
    left = Boxed
        value: 40
    right = Boxed
        value: 2
    result = left %% right
    result.println!
```

The receiver is `Boxed`, the right operand is `Boxed`, and the selected implementation returns `42`. The compiler does not need to know a special meaning for `%%`; the trait declaration and implementation provide it.

## Unary Operators

Prefix operators use the same type-directed model. The standard library provides negation for numeric values and logical not for booleans where an implementation exists.

```rock
main = !->
    value = 7
    condition = true
    negative = -value
    opposite = !condition
    negative.println!
    opposite.println!
```

The output is `-7` and `false`. User-defined types can also implement the `Neg` and `Not` traits with `@-` and `@!` methods and an associated `Output` type.

## Operator Sections

Parenthesized operator sections create functions. `(+ 2)` waits for its left operand and then adds `2`.

```rock
increment: I64 -> I64
increment = (+ 2)

main = !->
    increment 5 .println!
```

`increment` has type `I64 -> I64`, and the output is `7`. Use an explicit lambda when the missing side or ownership is not obvious.

## Functional Operators

The prelude also exports standard-library operators for function application through a value, mapping, binding, and fallback. The function signatures and values determine the concrete carrier types.

```rock
increment: I64 -> I64
increment = value -> value + 1

keep_even: I64 -> Option I64
keep_even = value ->
    if value % 2 == 0
    then Option::Some value
    else Option::None

main = !->
    mapped = Option::Some 4 <&> increment
    bound = Option::Some 4 >>= keep_even
    fallback = Option::None <|> Option::Some 9
    mapped.unwrap_or 0 .println!
    bound.unwrap_or 0 .println!
    fallback.unwrap_or 0 .println!
```

The output is `5`, `4`, and `9`. `<&>` maps a function over `Option`, `>>=` calls a function that returns another `Option`, `<|>` chooses the first present value, and `|>` passes a value to a function. These meanings are standard-library definitions, not compiler fallbacks.

## Parsing and Common Mistakes

Function application has precedence `8`. Operators above that precedence become part of the final argument, while operators at or below it operate on the call result. This keeps arithmetic arguments concise and functional composition outside the call.

```rock
double: I64 -> I64
double = value -> value * 2

main = !->
    doubled_sum = double 2 + 2
    adjusted_result = (double 2) + 2
    doubled_sum .println!
    adjusted_result .println!
    Option::Some 2 <&> (+ 2) .unwrap_or 0 .println!
```

`+` has precedence `9`, so `double 2 + 2` means `double (2 + 2)` and prints `8`. Parentheses make `(double 2) + 2` apply addition to the call result and print `6`. `<&>` has precedence `8`, so it operates on `Option::Some 2`; the spaced dots then apply `unwrap_or` and `println!` to each complete result. A tight dot such as `value.method!` still binds directly to `value`.

An adjacent `&` starts a borrowed call argument: `read &value` means `read (&value)`, whereas a spaced `&` can be an infix operator. A trailing `?` after that argument applies to the whole call: `read &value?` means `(read (&value))?`, not `read (&(value?))`. See the complete [borrowed-call example](../functional/error-handling.md#borrowed-call-arguments). Parenthesize an argument explicitly when propagation should happen inside it.

Operators are library design. Use a symbol for a compact operation with a stable, documented reading direction; use a named function for a domain action whose meaning is not obvious from punctuation.

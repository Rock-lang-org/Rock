# Generics

Generics let one declaration work for many concrete types while preserving static checking. A generic declaration is checked once, then specialized for the concrete types used by the program.

## Generic Data Types

Type parameters follow the declaration name. A generic struct can store a value of its parameter type, and a generic enum can carry one of several parameter types.

```rock
struct Wrapper T
    < value: T

enum Choice A, B
    First A
    Second B

main = !->
    number = Wrapper
        value: 7
    text: Choice I64, &Str = Choice::Second "rock"
    number.value.println!
    match text
        Choice::First value => value.println!
        Choice::Second value => value.println!
```

The concrete type of `number` is `Wrapper I64`, so `number.value` is `I64`. The concrete type of `text` is `Choice I64, &Str`, and its selected payload is `&Str`. The annotation on `text` is needed because `Second` provides no value from which to infer the `First` payload type. The output is `7` and `rock`.

Generic arguments are written with spaces and commas, not angle brackets: `Wrapper I64` and `Choice I64, &Str`.

## Generic Functions

An unconstrained type parameter can be passed through a function without the function knowing its concrete representation.

```rock
identity: T -> T
identity = value -> value

main = !->
    number = identity 42
    flag = identity true
    number.println!
    flag.println!
```

At the first call, inference chooses `T = I64`; at the second, it chooses `T = Bool`. The output is `42` and `true`. The signature says that the result has exactly the same type as the argument.

## Trait Bounds

A generic body may only use operations promised by its bounds. Put bounds in a `where` clause.

```rock
struct Box T
    < value: T

same_box: Box T -> Box T -> Bool where T: Eq
same_box = left, right -> left.value == right.value

main = !->
    left = Box
        value: 7
    left_again = Box
        value: 7
    equal = Box
        value: 7
    different = Box
        value: 9
    same_box left, equal .println!
    same_box left_again, different .println!
```

The body uses `==`, so `T: Eq` is required. In both calls inference chooses `T = I64`; the output is `true` and `false`. Without the bound, the generic body would have no promise that `==` exists.

The same rule applies to named methods. Here `T: Show` supplies `show!`, and the concrete argument type must implement that capability.

```rock
show_value: &T -> String where T: Show
show_value = value -> value.show!

main = !->
    number = 42
    show_value &number .println!
```

Here `T` is inferred as `I64`; `I64` implements `Show`, and `show_value` returns a `String` containing `42`.

## Generic Implementations

An implementation can itself contain type parameters. This array implementation is selected only for arrays of length three, then its `T` is specialized from the receiver.

```rock
trait EchoArray
    @echo: I64 -> I64

impl EchoArray for [T; 3]
    @echo = value -> value

main = !->
    values = [7, 8, 9]
    values.echo 7 .println!
```

The receiver establishes `T = I64` and matches the required array length of three. The trait declares the argument and result of `echo` as `I64` independently of `T`; the method returns its argument, so the output is `7`. A generic implementation is not a promise that every possible instantiation works; its own bounds and receiver shape still have to match.

## Type Aliases

An alias gives an existing type another spelling. It does not create a new nominal type or change ownership.

```rock
type Number = I64

identity: Number -> Number
identity = value -> value

main = !->
    identity 42 .println!
```

`Number` is an alias for `I64`, so `identity` accepts and returns `I64`. The output is `42`. Use a struct or enum when a distinct type is required.

## Associated Types

An associated type is chosen by each trait implementation. Callers can use `Self::Output` in a trait signature without repeating an output type as another generic parameter.

```rock
trait Projector
    type Output
    @project: Self::Output -> Self::Output

struct Identity

impl Projector for Identity
    type Output = I64
    @project = value -> value

main = !->
    identity = Identity
    identity.project 13 .println!
```

The implementation fixes `Identity::Output` to `I64`, so the argument and result of `project` are both `I64`. The output is `13`. An implementation must define every associated type required by its trait.

## Constructor Parameters

Ordinary generics abstract over complete types such as `I64` or `Option I64`. Higher-kinded generics abstract over constructors such as `Option` or `Result _, I64`; the exact tested syntax is `F _` in a `where` bound. [Higher-Kinded Types](../functional/higher-kinded-types.md) begins with complete `Option`, `Result`, and `Vec` programs using that syntax. This chapter intentionally does not introduce a second spelling before those examples.

## Common Mistakes and Limits

Do not call a method or operator in a generic body without a bound that supplies it. Do not leave a `Result` error parameter implicit: write `Result I64, I64` or another concrete error type. Generic aliases and associated types preserve the underlying ownership rules; they do not make an owned value copyable or a borrowed value owned.

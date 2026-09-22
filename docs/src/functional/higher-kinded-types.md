# Higher-Kinded Types

Ordinary generics abstract over a complete type such as `I64`. Higher-kinded types abstract over a type constructor such as `Option`, `Vec`, or a partially applied `Result`. Rock's prelude provides standalone functions and consuming receiver methods for working with these constructors; ordinary calls do not need a qualified trait path.

## Kinds and Constructor Holes

The notation below is type-level documentation, not a Rock program:

```text
I64               : Type
Option            : Type -> Type
Result            : Type -> Type -> Type
Result _, I64     : Type -> Type
```

`Option` waits for one type argument. `Result _, I64` fixes the error type to `I64` and waits for its success type. The underscore is a constructor hole, not a runtime value.

## Functor and `F _`

The `Functor` trait maps a callback over a constructor while preserving its shape. The bound `F _: Functor` says that `F` is a unary constructor with a `Functor` implementation.

```rock
map_any: M -> F A -> F B where F _: Functor, M: FnMut A, B
map_any = mapper, value -> fmap mapper, value

increment: I64 -> I64
increment = value -> value + 1

double: I64 -> I64
double = value -> value * 2

make_values: () -> Vec I64
make_values = ->
    mut values = Vec::new!
    values.push 1
    values.push 2
    values.push 3
    values

main = !->
    option_result = map_any increment, Option::Some 4
    vector_result = make_values!.fmap double
    option_result.show!.println!
    vector_result.show!.println!
```

At the first call, `F = Option`, `A = I64`, and `B = I64`; at the second, the receiver supplies `F = Vec`. The standalone `fmap mapper, value` and receiver form `value.fmap mapper` delegate to the same constructor's `Functor` implementation. The output is `Some(5)` and `[2, 4, 6]`. Both input carriers are consumed by their mapping operation.

For `Result`, a function signature can fix the error type while the call still infers the constructor:

```rock
map_result: M -> Result A, I64 -> Result B, I64 where M: FnMut A, B
map_result = mapper, value -> fmap mapper, value

increment: I64 -> I64
increment = value -> value + 1

main = !->
    success = map_result increment, Result::Ok 4
    failure = map_result increment, Result::Err 9
    success.show!.println!
    failure.show!.println!
```

Here the inferred constructor is `Result _, I64`: the error type stays fixed while `fmap` changes the success payload. It has the required unary kind, while bare `Result` would still require two arguments. The output is `Ok(5)` and `Err(9)`. When explicit dispatch is useful, the same operation can be written `(Result _, I64)::Functor::fmap mapper, value`; that qualification is not required at ordinary call sites.

## Applicative Values

`Applicative` adds `pure` for lifting a value and `ap` for applying a wrapped function to a wrapped argument.

```rock
repure_any: F I64 -> F I64 where F _: Applicative
repure_any = ignored -> pure 2

apply_any: F (I64 -> I64) -> F I64 -> F I64 where F _: Applicative
apply_any = wrapped_function, wrapped_value ->
    ap wrapped_function, wrapped_value

increment: I64 -> I64
increment = value -> value + 1

main = !->
    lifted = repure_any Option::Some 0
    wrapped = Option::Some increment
    applied = apply_any wrapped, lifted
    applied.show!.println!
```

The inferred constructor is `F = Option`. `repure_any` ignores its input carrier and calls `pure 2`, producing `Some 2`; then `ap` applies `increment`, producing `Some 3`. The output is `Some(3)`. The receiver form is `wrapped_function.ap wrapped_value`; its receiver is the carrier holding the function.

Unlike `fmap`, `pure` has no input carrier from which to infer its constructor. Its expected return type must supply that information. In `repure_any`, the return signature supplies `F I64`, and the caller fixes `F` through the argument. A concrete return signature works too:

```rock
make_option: () -> Option I64
make_option = -> pure 2

make_result: () -> Result I64, I64
make_result = -> pure 3

main = !->
    make_option!.show!.println!
    make_result!.show!.println!
```

This prints `Some(2)` and `Ok(3)`. Without such context, `pure 2` cannot choose a carrier. `pure` remains a standalone or associated function because it creates a carrier rather than consuming an existing one.

## Monad Binding

`Monad` sequences a carrier with a callback that returns another value of the same constructor.

```rock
bind_any: F I64 -> M -> F I64 where F _: Monad, M: FnMut I64, (F I64)
bind_any = value, callback -> value.bind callback

add_two: I64 -> Option I64
add_two = value -> Option::Some value + 2

main = !->
    start = Option::Some 3
    result = bind_any start, add_two
    result.show!.println!
```

Here `F = Option` and `M` is the callback type `I64 -> Option I64`. `bind` unwraps `Some 3`, calls `add_two`, and returns `Some 5`. The output is `Some(5)`; a `None` input would skip the callback. The standalone spelling is `bind value, callback`.

## Foldable and Traversable

`Foldable` consumes a constructor from left to right with a state and a step function. `Traversable` maps every element into an effect while rebuilding the original shape inside that effect.

```rock
fold_digits: (I64, I64) -> I64
fold_digits = pair -> pair.0 * 10 + pair.1

increment_effect: I64 -> Option I64
increment_effect = value -> Option::Some value + 1

make_values: () -> Vec I64
make_values = ->
    mut values = Vec::new!
    values.push 1
    values.push 2
    values.push 3
    values

main = !->
    option_total = foldl fold_digits, 0, Option::Some 4
    vector_total = make_values!.foldl fold_digits, 0
    traversed = make_values!.traverse increment_effect
    option_total.println!
    vector_total.println!
    traversed.show!.println!
```

The fold over `Some 4` returns `4`; the vector fold computes `123`; and traversal produces `Some([2, 3, 4])`. The output is `4`, `123`, and `Some([2, 3, 4])`. `Vec` supplies `Functor`, `Foldable`, and `Traversable`; its traversal consumes the input vector while constructing a new vector. The standalone traversal spelling is `traverse increment_effect, make_values!`.

`traverse_m` uses monadic binding instead of applicative application and can stop as soon as an effect fails.

```rock
stop_at_two: I64 -> Option I64
stop_at_two = value ->
    if value == 2
        Option::None
    else
        Option::Some value

make_values: () -> Vec I64
make_values = ->
    mut values = Vec::new!
    values.push 1
    values.push 2
    values.push 3
    values

main = !->
    result = make_values!.traverse_m stop_at_two
    result.show!.println!
```

The second callback returns `None`, so traversal stops and the output is `None`. The standalone spelling is `traverse_m stop_at_two, make_values!`.

## Traversing for Effects

Use the standalone `for_each values, action` when the callback exists for effects rather than to build another container. The action returns `()` and may mutate its captures through `FnMut`:

```rock
main = !->
    mut total = 0
    for_each 1..=3, number !->
        total = total + number
    total.println!

    for_each (Option::Some 7), number !-> number.println!
    result: Result I64, I64 = Result::Ok 9
    for_each result, number !-> number.println!
```

The output is `6`, `7`, and `9`. `Option::None` and `Result::Err` invoke no callback. A `Vec T` supplies each owned element in order and is consumed; `&Vec T` and `&[T]` supply shared references while preserving the owner. Borrow an array as a slice to use the same function.

The public function uses the ordinary `ForEach T` bridge trait. Its blanket implementation for `F T where F _: Foldable` derives traversal from `F::Foldable::foldl`: the callback is carried as fold state, called once per element, and returned for the next step. Any new constructor implementing `Foldable` receives this behavior automatically; no separate `ForEach` implementation is needed.

Native `Range` is a concrete integer type, not a unary type constructor. Its dedicated bridge implementation visits bounded ranges directly without allocating a vector. Exclusive `start..end` and inclusive `start..=end` ranges run in ascending order; reversed ranges are empty, and an inclusive equal-endpoint range visits one integer. Open-ended ranges terminate with `for_each requires a bounded range` before invoking the action.

## Sequencing Effects

`sequence` is the standard `Traversable` operation for turning `T (G A)` into `G (T A)`. The following exact stdlib pattern sequences a vector of options.

```rock
main = !->
    mut effects = Vec::new!
    effects.push Option::Some 4
    effects.push Option::Some 5
    successful = sequence effects
    match successful
        Option::Some values =>
            values.len!.println!
            match values.get 0
                Option::Some value => *value .println!
                Option::None => 0.println!
        Option::None => 0.println!

    mut failed_effects = Vec::new!
    failed_effects.push Option::Some 1
    failed_effects.push Option::None
    failed_effects.push Option::Some 3
    failed = failed_effects.sequence!
    match failed
        Option::Some values => values.len!.println!
        Option::None => -1 .println!
```

The successful sequence prints `2` and `4`. The sequence containing `None` prints `-1`, because one missing effect makes the whole `Option` result absent. Both `sequence values` and `values.sequence!` consume the input vector.

## Choosing a Call Style

The standalone functions and receiver methods are available through the prelude:

| Standalone | Receiver |
| --- | --- |
| `fmap mapper, value` | `value.fmap mapper` |
| `ap wrapped_function, wrapped_value` | `wrapped_function.ap wrapped_value` |
| `bind value, callback` | `value.bind callback` |
| `foldl step, initial, values` | `values.foldl step, initial` |
| `traverse action, values` | `values.traverse action` |
| `traverse_m action, values` | `values.traverse_m action` |
| `sequence values` | `values.sequence!` |

The receiver APIs are ordinary blanket trait implementations on fully applied values. Implementing the corresponding constructor traits gives a custom carrier these methods automatically. They consume their receivers and preserve the callback bounds of the constructor operations, including `FnMut` where supported. Qualified calls such as `F::Functor::fmap` remain useful inside implementations or when choosing a trait explicitly.

For example, this custom carrier only implements `Functor`; the prelude supplies its `.fmap` method:

```rock
enum Parcel T
    Item T

impl Functor for Parcel
    fmap = mut mapper, value ->
        match value
            Parcel::Item item => Parcel::Item (mapper item)

increment: I64 -> I64
increment = value -> value + 1

main = !->
    parcel = Parcel::Item 4
    match parcel.fmap increment
        Parcel::Item value => value.println!
```

This prints `5`. The standalone `fmap` function uses the same implementation.

## Current Limits and Design Guidance

Use `F _` for a unary constructor bound and `Result _, E` when fixing one `Result` parameter. Do not pass a bare binary `Result` where a unary constructor is required. The current stdlib intentionally gives `Vec` no `Applicative` or `Monad` implementation because Cartesian and zipped list semantics would make an arbitrary choice. Existing `.map` and `.and_then` methods remain useful for concrete containers; `fmap` and `bind` provide a shared vocabulary for constructor-generic code.

The examples here use constructor holes, standalone functions, and receiver methods. Explicit type-lambda syntax is not needed for these APIs.

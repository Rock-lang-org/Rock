# Higher-Kinded Types

Ordinary generics abstract over a complete type such as `I64`. Higher-kinded types abstract over a type constructor such as `Option`, `Vec`, or a partially applied `Result`. This chapter uses the exact constructor-hole syntax supported by the current stdlib and integration tests.

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
map_any = mapper, value -> F::Functor::fmap mapper, value

increment: I64 -> I64
increment = value -> value + 1

double: I64 -> I64
double = value -> value * 2

make_values: () -> Vec I64
make_values = ->
    mut values: Vec I64 = Vec::new!
    values.push 1
    values.push 2
    values.push 3
    values

main = !->
    option_result: Option I64 = map_any increment, Option::Some 4
    vector_result: Vec I64 = map_any double, make_values!
    option_result.show!.println!
    vector_result.show!.println!
```

At the first call, `F = Option`, `A = I64`, and `B = I64`; at the second, `F = Vec`. `map_any` calls the selected constructor's `Functor::fmap`, not a compiler special case. The output is `Some(5)` and `[2, 4, 6]`. Both input carriers are consumed by their mapping operation.

The same constructor can be named explicitly when a partially applied type is needed.

```rock
map_result: M -> Result A, I64 -> Result B, I64 where M: FnMut A, B
map_result = mapper, value -> (Result _, I64)::Functor::fmap mapper, value

increment: I64 -> I64
increment = value -> value + 1

main = !->
    success: Result I64, I64 = map_result increment, Result::Ok 4
    failure: Result I64, I64 = map_result increment, Result::Err 9
    success.show!.println!
    failure.show!.println!
```

`Result _, I64` has the required unary kind, while bare `Result` would still require two arguments. The output is `Ok(5)` and `Err(9)`.

## Applicative Values

`Applicative` adds `pure` for lifting a value and `ap` for applying a wrapped function to a wrapped argument.

```rock
repure_any: F I64 -> F I64 where F _: Applicative
repure_any = ignored -> F::Applicative::pure 2

apply_any: F (I64 -> I64) -> F I64 -> F I64 where F _: Applicative
apply_any = wrapped_function, wrapped_value ->
    F::Applicative::ap wrapped_function, wrapped_value

increment: I64 -> I64
increment = value -> value + 1

main = !->
    lifted: Option I64 = repure_any Option::Some 0
    wrapped: Option (I64 -> I64) = Option::Some increment
    applied: Option I64 = apply_any wrapped, lifted
    applied.show!.println!
```

The inferred constructor is `F = Option`. `repure_any` ignores its input carrier and calls `pure 2`, producing `Some 2`; then `ap` applies `increment`, producing `Some 3`. The output is `Some(3)`.

## Monad Binding

`Monad` sequences a carrier with a callback that returns another value of the same constructor.

```rock
bind_any: F I64 -> M -> F I64 where F _: Monad, M: FnMut I64, (F I64)
bind_any = value, callback -> F::Monad::bind value, callback

add_two: I64 -> Option I64
add_two = value -> Option::Some value + 2

main = !->
    start: Option I64 = Option::Some 3
    result: Option I64 = bind_any start, add_two
    result.show!.println!
```

Here `F = Option` and `M` is the callback type `I64 -> Option I64`. `bind` unwraps `Some 3`, calls `add_two`, and returns `Some 5`. The output is `Some(5)`; a `None` input would skip the callback.

## Foldable and Traversable

`Foldable` consumes a constructor from left to right with a state and a step function. `Traversable` maps every element into an effect while rebuilding the original shape inside that effect.

```rock
fold_digits: (I64, I64) -> I64
fold_digits = pair -> pair.0 * 10 + pair.1

increment_effect: I64 -> Option I64
increment_effect = value -> Option::Some value + 1

make_values: () -> Vec I64
make_values = ->
    mut values: Vec I64 = Vec::new!
    values.push 1
    values.push 2
    values.push 3
    values

main = !->
    option_total: I64 = Option::Foldable::foldl fold_digits, 0, Option::Some 4
    vector_total: I64 = Vec::Foldable::foldl fold_digits, 0, make_values!
    traversed: Option (Vec I64) = Vec::Traversable::traverse increment_effect, make_values!
    option_total.println!
    vector_total.println!
    traversed.show!.println!
```

The fold over `Some 4` returns `4`; the vector fold computes `123`; and traversal produces `Some([2, 3, 4])`. The output is `4`, `123`, and `Some([2, 3, 4])`. `Vec` supplies `Functor`, `Foldable`, and `Traversable`; its traversal consumes the input vector while constructing a new vector.

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
    mut values: Vec I64 = Vec::new!
    values.push 1
    values.push 2
    values.push 3
    values

main = !->
    result: Option (Vec I64) = Vec::Traversable::traverse_m stop_at_two, make_values!
    result.show!.println!
```

The second callback returns `None`, so traversal stops and the output is `None`.

## Traversing for Effects

Use the standalone `for_each values, action` when the callback exists for effects rather than to build another container. The action returns `()` and may mutate its captures through `FnMut`:

```rock
main = !->
    mut total: I64 = 0
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
    mut effects: Vec (Option I64) = Vec::new!
    effects.push Option::Some 4
    effects.push Option::Some 5
    successful: Option (Vec I64) = sequence effects
    match successful
        Option::Some values =>
            values.len!.println!
            match values.get 0
                Option::Some value => *value .println!
                Option::None => 0.println!
        Option::None => 0.println!

    mut failed_effects: Vec (Option I64) = Vec::new!
    failed_effects.push Option::Some 1
    failed_effects.push Option::None
    failed_effects.push Option::Some 3
    failed: Option (Vec I64) = sequence failed_effects
    match failed
        Option::Some values => values.len!.println!
        Option::None => -1 .println!
```

The successful sequence prints `2` and `4`. The sequence containing `None` prints `-1`, because one missing effect makes the whole `Option` result absent. `sequence` consumes both input vectors.

## Current Limits and Design Guidance

Use `F _` for a unary constructor bound and `Result _, E` when fixing one `Result` parameter. Do not pass a bare binary `Result` where a unary constructor is required. The current stdlib intentionally gives `Vec` no `Applicative` or `Monad` implementation because Cartesian and zipped list semantics would make an arbitrary choice. Ordinary `Option::map` or `Vec::map` is often clearer than a constructor-generic helper used once.

The examples here use tested constructor applications and associated trait calls. Explicit type-lambda syntax and custom higher-kinded carriers are not presented as stable teaching forms until a user-facing integration example establishes their complete syntax and ownership behavior.

# Bindings, Reassignment, and Mutability

A binding gives a value a name. Rock does not use a `let` keyword; an assignment-shaped statement introduces a local when the name has not been seen in the current scope.

```rock
main = !->
    answer = 42
    name = "Ada"
    answer.println!
    name.println!
```

The compiler infers `answer` as `I64` and `name` as `&Str`. The names are available only after their declarations. The `println!` calls are evaluated for effects; `!->` discards the trailing value and makes `main` return unit (`()`), which maps to process exit status `0`.

## Type annotations

Put an annotation after the name and before `=`:

```rock
main = !->
    answer: I64 = 42
    ratio: F64 = 0.75
    enabled: Bool = true
    answer.println!
    ratio.println!
    enabled.println!
```

An annotation documents the intended type and gives inference an expected type. It is not a conversion: `0.75` already has a floating-point type in this example.

## Observed reassignment behavior

The current compiler permits a later assignment to an existing local without requiring `mut`:

```rock
main = !->
    count = 0
    count = count + 1
    count = count + 1
    count.println!
```

The first `count = 0` establishes the local. The next two assignments target that local and update its value. This is current observed behavior, not a promise that every future mutability rule will be identical.

`mut` is required where the operation explicitly requests a mutable place. A mutable borrow is the clearest example:

```rock
main = !->
    mut value = 1
    reference = &mut value
    *reference = 2
    value.println!
```

`value` must be marked `mut` because `&mut value` creates a mutable reference. The assignment through `*reference` changes the original place, so the output is `2`.

The same requirement applies to a mutable receiver method. `^@` marks a method that borrows its receiver mutably:

```rock
struct Counter
    < value: I64

impl Counter
    ^@increment = amount ->
        self.value = self.value + amount
        return

main = !->
    mut counter = Counter
        value: 3
    counter.increment 4
    counter.value.println!
```

`counter` is marked `mut` because `increment` needs a mutable receiver. The method changes the field from `3` to `7`. Ordinary reassignment and mutable borrowing are separate checks: the former currently works without `mut`, while the latter requires it.

## Reusing a name during a transformation

A parameter can be assigned a new value as a transformation step:

```rock
increment_and_print = value ->
    value = value + 1
    value.println!
    value

main = !->
    result = increment_and_print 4
    result.println!
```

The assignment changes the current value of `value`; it does not introduce a second parameter. Keeping the same name is useful when the old representation is no longer needed, but a new descriptive name is clearer when both values remain meaningful.

## Destructuring

A pattern on the left can bind several parts of a tuple:

```rock
main = !->
    pair = (10, "ten")
    (number, word) = pair
    number.println!
    word.println!
```

`pair = (10, "ten")` creates the tuple. In `(number, word) = pair`, the pattern binds that tuple's first element to `number` and its second element to `word`. The pattern must have a shape compatible with the value.

Patterns can mark an individual binding as mutable. Here only `left` is marked `mut`; the plain reassignment shown would also work without that marker:

```rock
main = !->
    (mut left, right) = (1, 2)
    left = left + right
    left.println!
```

The output is `3`, while `right` remains `2`. The matching chapter shows how an enum pattern can both check a variant and bind its payloads.

## Updating a field

A field path is also an assignment target. Declare the struct in the same module as the use:

```rock
struct Point
    < x: I64
    < y: I64

main = !->
    point = Point
        x: 1
        y: 2
    point.x = 5
    point.x.println!
    point.y.println!
```

The first assignment constructs `point`; `point.x = 5` updates its existing field. Indexing and mutable receiver calls use the same general idea of selecting a place, but their trait and receiver contracts can impose additional requirements.

## Unit-valued functions

Some functions exist for effects and return `()`. A unit-arrow body uses `!->`:

```rock
log_value: I64 -> ()
log_value = value !->
    value.println!

main = !->
    log_value 9
```

A `!->` body evaluates every statement, including its trailing expression, then discards the trailing value and returns `()` automatically. Use an ordinary `->` body when the trailing value should become the function result.

## Common mistakes

- Saying that every assignment requires `mut`; plain reassignment currently does not.
- Forgetting `mut` before a value passed to `&mut` or a `^@` receiver.
- Destructuring a tuple with the wrong number of elements.
- Using a field or enum name without declaring its containing type in the module.

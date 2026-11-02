# Structs

Structs combine named fields into a new type. They are useful when several values form one concept and should travel through a function together.

## Declaring a struct

```rock
struct Point
    < x: I64
    < y: I64
```

The declaration introduces `Point`. Each indented line declares one field. The leading `<` exports a field from its module; an unmarked field is private to that module.

Generic parameters follow the type name:

```rock
struct Pair A, B
    < first: A
    < second: B
```

`Pair A, B` is one generic struct declaration. Each field uses one of the declared type parameters.

## Constructing a value

Write the type name followed by an indented field block:

```rock
struct Point
    < x: I64
    < y: I64

main = !->
    origin = Point
        x: 0
        y: 0
    %println origin.x, "\n", origin.y
```

The field labels are part of the constructor syntax. Rock checks that every required field is present and that each expression has the declared type. Initialize every field explicitly; default field expressions are not reliably preserved by the current semantic pipeline.

Generic fields can often be inferred from their constructor expressions:

```rock
struct Pair A, B
    < first: A
    < second: B

main = !->
    entry = Pair
        first: "answer"
        second: 42
    %println entry.first, "\n", entry.second
```

Here the compiler infers `A` as `&Str` and `B` as `I64` from the two field values.

## Reading and updating fields

Use dot notation to read a field and to select a field assignment target:

```rock
struct Point
    < x: I64
    < y: I64

main = !->
    point = Point
        x: 2
        y: 3
    horizontal = point.x
    point.x = 8
    %println horizontal, "\n", point.x
```

`horizontal` keeps the value read before the update, so the output is `2` followed by `8` on the next line. Whether a field read copies, borrows, or moves depends on the field type and the surrounding ownership context.

## Adding behavior

An inherent `impl` groups associated functions and methods with a type:

```rock
struct Point
    < x: I64
    < y: I64

impl Point
    new: I64 -> I64 -> Point
    new = x, y ->
        Point
            x: x
            y: y

    @length_squared: I64
    @length_squared = -> @x * @x + @y * @y

main = !->
    point = Point::new 3, 4
    value = point.length_squared!
    value.println!
```

`Point::new` is an associated function because it is called through the type path. `length_squared!` is a method because it is called through a value. `@` marks a shared receiver, and `@x` is shorthand for reading the receiver's field.

Mutable and consuming receiver markers are `^@` and `~@`:

```rock
struct Counter
    < value: I64

impl Counter
    ^@increment = amount ->
        self.value = self.value + amount
        return

    ~@take = -> self.value

main = !->
    mut counter = Counter
        value: 2
    counter.increment 5
    total = counter.take!
    total.println!
```

The `mut` binding is required for `increment` because its receiver is mutable. `take!` consumes the value, so `counter` cannot be used afterward.

## Visibility

Export a declaration by placing `<` before it. Exported fields are independently marked:

```rock
< struct PublicPoint
    < x: I64
    < y: I64

main = !->
    point = PublicPoint
        x: 1
        y: 2
    point.x.println!
```

An explicit export path can also assemble a module facade, but inline exports are easiest to read for a small public type.

## Common mistakes

- Omitting a required field or using the wrong field type.
- Treating `Point::new` and `point.new` as the same call; the former is associated, the latter is a method-style call.
- Forgetting that `@`, `^@`, and `~@` describe different receiver ownership contracts.
- Calling a mutable receiver method without a `mut` binding.
- Assuming field defaults are reliable in the current compiler.

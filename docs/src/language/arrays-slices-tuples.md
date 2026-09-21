# Arrays, Slices, and Tuples

Rock has three useful shapes for a fixed collection of values:

- `[T; N]` is an owned, fixed-size array. `T` is the element type and `N` is part of the type.
- `&[T]` is a shared borrowed slice. It views contiguous storage without owning it.
- `(A, B)` and longer tuples are fixed products. Each position can have a different type.

`Vec T` is the standard library's growable owned collection. Choose it when the number of elements changes at run time.

## Fixed-Size Arrays

An array literal infers one element type and one length. The following program creates an array, annotates another array explicitly, and prints both values.

```rock
main = !->
    scores = [10, 20, 30, 40]
    labeled: [I64; 4] = [10, 20, 30, 40]
    scores.println!
    labeled.println!
```

The inferred type of `scores` is `[I64; 4]`: integer literals are inferred as `I64` here, and the literal has four elements. The result is:

```text
[10, 20, 30, 40]
[10, 20, 30, 40]
```

All elements must have the same type. A cast or an annotation can establish a narrower element type.

```rock
main = !->
    bytes: [U8; 3] = [97, 98, 99]
    first: I64 = bytes[0] as I64
    first.println!
```

The type of `bytes` is `[U8; 3]`, and `first` is `I64` because the `as I64` cast is explicit. The output is `97`.

### Repeat Literals

`[value; length]` evaluates `value` once and copies the result into a fixed-size array. The initializer therefore must be a value the compiler knows is copyable.

```rock
main = !->
    zeros: [U8; 4] = [0; 4]
    zeros[0].println!
    zeros[3].println!
```

The output is two lines containing `0`. A `String` is owned and is not copyable, so a repeated owned string is rejected. Use a loop and a fresh `String::from_str` call when each element must own a separate allocation.

### Indexing and Mutation

Reading `values[index]` selects the standard library's `Index` implementation. Assigning `values[index] = replacement` selects `IndexMut` and requires a mutable place.

```rock
main = !->
    mut values: [I64; 3] = [10, 20, 30]
    before = values[1]
    values[1] = 99
    after = values[1]
    before.println!
    after.println!
```

`before` and `after` are both inferred as `I64`; the output is `20` followed by `99`. The array remains owned by `values` before and after the indexing operations. An index below zero or at least the array length terminates the program with an `index out of bounds` message.

Nested arrays use the same rule at each position.

```rock
set_second: &mut [I64] -> ()
set_second = row ->
    row[1] = 7
    return

first_row: &[I64] -> I64
first_row = row -> row[1]

main = !->
    mut row: [I64; 2] = [1, 0]
    second_row: [I64; 2] = [0, 1]
    set_second &mut row
    first_row &row .println!
    first_row &second_row .println!
    matrix: [[I64; 2]; 2] = [row, second_row]
```

The output is `7` and `1`. Nested fixed-array literals are valid, but the current implementation does not support chained indexing directly on an inner fixed-array value; borrow a row as a slice before indexing when a program needs that operation.

## Slices

`[T]` is a dynamically sized view, not a standalone value type for fields or parameters. In ordinary safe code, write it behind a reference: `&[T]` for shared access or `&mut [T]` for exclusive mutable access.

Borrowing a fixed array as a slice does not move the array. The slice points at the same storage, so the owner remains responsible for the array.

```rock
sum_three: &[I64] -> I64
sum_three = values ->
    values[0] + values[1] + values[2]

main = !->
    values: [I64; 3] = [2, 4, 6]
    total: I64 = sum_three &values
    total.println!
    values[0].println!
```

Before the call, `values` owns `[2, 4, 6]`. During the call, `&values` is coerced to `&[I64]`; `sum_three` borrows it and does not own it. After the call, `values` is still available. The output is `12` and `2`.

A fixed array value is not coerced to a slice value by itself. Borrow it explicitly at the call site. This keeps the lifetime of the view tied to the array and makes ownership visible.

Mutable slices let a function update the owner's storage without taking ownership of the array.

```rock
write_middle: &mut [I64] -> ()
write_middle = values ->
    values[1] = 42
    return

main = !->
    mut values: [I64; 3] = [7, 8, 9]
    write_middle &mut values
    values[1].println!
```

Before the call, `values` is mutable and owned by `main`. During the call, `&mut values` gives `write_middle` the only active mutable access. After the call, the mutable borrow ends at its last use and `values[1]` prints `42`.

Slices support the same indexing traits as arrays. A slice does not resize, allocate, or own the backing storage; use `Vec T` for those operations.

### Range Slicing

Use a range to borrow a prefix, suffix, or middle portion of an array or slice. Range indexing selects a view through `Index` or `IndexMut`; the surrounding `&` or `&mut` makes the borrow explicit. No elements are copied or allocated.

```rock
> stdlib::slice::slice_len

inspect: &[I64] -> ()
inspect = values !->
    (slice_len values).println!
    values[0].println!

main = !->
    values = [10, 20, 30, 40]
    inspect (&values[..2])
    inspect (&values[1..])
    inspect (&values[1..=2])
```

The example prints lengths and first values for `[10, 20]`, `[20, 30, 40]`, and `[20, 30]`. `slice_len` returns an `I64` element count without requiring a compiler intrinsic.

For a four-element sequence, the six range forms select these positions:

| Range | Meaning | Positions |
| --- | --- | --- |
| `1..3` | Start included, end excluded | 1, 2 |
| `1..=3` | Both endpoints included | 1, 2, 3 |
| `..2` | Prefix ending before 2 | 0, 1 |
| `..=2` | Prefix through 2 | 0, 1, 2 |
| `2..` | Suffix starting at 2 | 2, 3 |
| `..` | Whole sequence | 0, 1, 2, 3 |

Ranges are first-class values, so a program can store one before indexing:

```rock
main = !->
    values = [10, 20, 30, 40]
    middle = 1..3
    view = &values[middle]
    (*view)[0].println!
```

These values have the standard library type `Range`, whose endpoints are `I64`. Creating a range does not check it against any collection; indexing does.

### Bounds and Empty Views

An exclusive range must satisfy `0 <= start <= end <= length`. Equal endpoints produce an empty view, including `length..length` at the end of a sequence. An inclusive endpoint must identify an actual element, so it must be less than the length. `..=length` is not a spelling for the whole sequence; use `..` instead.

```rock
> stdlib::slice::slice_len

main = !->
    values = [10, 20, 30, 40]
    empty = &values[4..4]
    (slice_len empty).println!
```

The output is `0`. Do not index element zero of an empty slice. Negative, reversed, or out-of-bounds ranges terminate the process with `range out of bounds`; slicing does not clamp endpoints or return an `Option`. Validate a count from untrusted input before constructing its view. A byte count returned by a successful standard-library read already fits the supplied buffer, which makes `&buffer[..count]` useful for [safe I/O](../stdlib/io-and-files.md).

### Mutable Subslices

A mutable range view updates the original storage:

```rock
update_middle: &mut [I64] -> ()
update_middle = values !->
    mut middle = &mut values[1..3]
    middle[0] = 77

main = !->
    mut values = [10, 20, 30, 40]
    update_middle (&mut values)
    values[1].println!
    values[2].println!
```

The output is `77` and `30`. Index zero in `middle` is index one in `values`. Shared and mutable views obey ordinary borrowing rules: a view cannot outlive its owner, and conflicting access is rejected while an earlier borrow remains live. Do not assume the borrow checker can prove that two independently written ranges are disjoint.

Range syntax also supports bounded integer loops:

```rock
main = !->
    mut total = 0
    for value in 1..=3
        total = total + value
    total.println!
```

This prints `6`; `1..3` would visit only `1` and `2`. The current range-loop form requires both endpoints directly in the loop expression; open-ended slicing syntax is not an unbounded iterator API.

## Growable Vectors

`Vec T` owns a heap allocation and can grow with `push`. Its receiver is mutable for mutation, and `len!` borrows it to inspect the current length.

```rock
main = !->
    mut values: Vec I64 = Vec::new!
    values.push 10
    values.push 20
    values.push 30
    values.len!.println!
    values[1].println!
```

The output is `3` and `20`. `values` owns all three elements before and after `push`; indexing borrows the selected element for the expression. `Vec::get` is the checked, non-panicking alternative when an index may be absent.

`Vec T` also supports the native range forms. A shared range produces `&[T]`, while a mutable range produces `&mut [T]`; neither operation copies the selected elements.

```rock
> stdlib::slice::slice_len

main = !->
    mut values: Vec I64 = Vec::new!
    values.push 10
    values.push 20
    values.push 30
    tail = &values[1..]
    (slice_len tail).println!
    tail[0].println!
```

This prints `2` and `20`. Do not resize or move the vector while a borrowed view is still in use. For a single optional element rather than a range, use `get`:

```rock
main = !->
    mut values: Vec I64 = Vec::new!
    values.push 10
    match values.get 4
        Option::Some value => *value .println!
        Option::None => -1 .println!
```

The output is `-1`. `get` returns `Option &I64`, so the `Some` binding is a shared reference and `*value` reads the integer without moving it.

## Tuples

A tuple stores a fixed number of values in a fixed order. Its type records every component.

```rock
main = !->
    entry: (I64, Bool, &Str) = (7, true, "rock")
    entry.0.println!
    entry.1.println!
    entry.2.println!
```

The annotation specifies component types `I64`, `Bool`, and `&Str`; the output is `7`, `true`, and `rock`. `.0`, `.1`, and `.2` are tuple projections, not array indexes, so their positions are checked statically.

Functions can return tuples and callers can project the result.

```rock
swap: I64 -> I64 -> (I64, I64)
swap = left, right -> (right, left)

main = !->
    pair: (I64, I64) = swap 10, 20
    pair.0.println!
    pair.1.println!
```

`swap` returns `(20, 10)`, so the output is `20` and `10`. A parenthesized expression such as `(7)` is grouping; a one-element tuple is not part of the current tuple syntax.

## Choosing a Type

Use `[T; N]` when the length is fixed and the data should be stored inline in the owning value. Use `&[T]` or `&mut [T]` when a function needs a temporary view. Use `Vec T` when the collection owns a variable number of elements. Use `(A, B)` when the positions have different meanings or types.

The current slice limitations are deliberate and important: a bare `[T]` is rejected in ordinary struct fields and function parameters, and a fixed array must be borrowed before it can satisfy a slice parameter. The current `Vec` API is the standard growable sequence; a slice cannot be resized.

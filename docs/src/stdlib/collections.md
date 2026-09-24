# Strings and Collections

The standard library supplies owned strings and heap-backed containers. Their APIs follow Rock's ownership model: shared methods inspect without taking ownership, mutable methods require an exclusive borrow, and consuming transformations move elements into a new result. The prelude exports `String`, `Vec`, `HashMap`, `Box`, `Arc`, and `Option`; specialized string search helpers are imported explicitly below.

## `String` and `&Str`

`&Str` is a borrowed string slice. `String` owns a null-terminated heap buffer. Construct an owner when data must outlive the expression that supplied the slice, and keep a slice when a function only needs to inspect text.

```rock
describe: &Str -> String
describe = value -> String::from value

main = !->
    name = "Rock"
    owned = describe name
    empty = String::new!
    number = String::from 42
    copy = owned.clone!
    length = owned.len!
    view = owned.as_str!
    combined = owned.concat number
    message = "Hello, " + "Rock" + "!"
    empty.len!.println!
    copy.println!
    length.println!
    view.println!
    combined.println!
    message.println!
```

The output is `0`, `Rock`, `4`, `Rock`, `Rock42`, and `Hello, Rock!`. `String::from` converts through the `From` trait, while `String::new!` constructs an empty string; `len!` reads a byte count and `as_str!` borrows a string view. `clone!` makes a second owner. `owned.concat number` borrows its receiver `owned`, consumes the argument `number`, and returns a new owner. Because `owned` remains alive, `view` can still be printed after concatenation. The `+` implementations cover `String` and `&Str` combinations and also return a new `String`.

Strings are byte-oriented in the current library. Lengths, searches, substrings, and byte indexes count bytes rather than Unicode scalar values. A multi-byte UTF-8 character must not be split by a byte range unless the caller intentionally handles raw bytes.

## Byte and search helpers

The search helpers live in `stdlib::string` and are not prelude names. This fence imports every non-prelude function it uses and keeps the byte buffer separate from the borrowed `&Str` search input.

```rock
> stdlib::string::byte_at
> stdlib::string::byte_substr
> stdlib::string::string_contains
> stdlib::string::string_find
> stdlib::string::string_len

main = !->
    text = "rock"
    length = string_len text
    offset = string_find text, "oc"
    contains = string_contains text, "ck"
    bytes = [114, 111, 99, 107]
    middle = byte_substr &bytes, 1, 2
    second = byte_at &bytes, 1
    length.println!
    offset.println!
    contains.println!
    middle.len!.println!
    second as I64 .println!
```

The output is `4`, `1`, `1`, `2`, and `111`. `string_find` returns a byte offset or `-1`; `string_contains` returns `1` or `0`; `byte_substr` returns an owned `Vec U8`; and `byte_at` reads one byte from a byte slice. Out-of-range behavior is not a Unicode-aware character operation, so validate byte boundaries in the caller.

## `Vec T`

`Vec T` is a growable owned sequence. A mutable receiver method requires a mutable binding. `get` returns `Option &T` for a checked lookup; `set` replaces an existing element; `swap_remove` removes an element by moving the last element into its position.

```rock
main = !->
    mut values = Vec::new!
    values.push 10
    values.push 20
    values.push 30
    view = values.as_slice!
    view.println!
    values.set 1, 99
    match values.get 1
        Option::Some value => value.println!
        Option::None => -1 .println!
    match values.swap_remove 0
        Option::Some value => value.println!
        Option::None => -1 .println!
    values.len!.println!
```

The output is `[10, 20, 30]`, `99`, `10`, and `2`. `as_slice!` borrows the vector, so do not keep that borrow live across `push`, `set`, or `swap_remove`. `swap_remove` does not preserve order: after removing index `0`, the old final element occupies that position.

## Consuming and borrowing transformations

The callback type tells you whether an operation moves elements or borrows them. `map`, `filter`, and `filter_map` consume the source; `map_ref`, `for_each`, and `retain` borrow elements while processing them. `try_map` consumes the source and stops at the first `Result::Err`.

```rock
main = !->
    mut source = Vec::new!
    source.push 1
    source.push 2
    source.push 3
    mapped = source.map (+ 1)

    mut borrowed_source = Vec::new!
    borrowed_source.push 4
    borrowed_source.push 5
    references = borrowed_source.map_ref (+ 1)
    borrowed_source.for_each (!.println!)

    mut filtered_source = Vec::new!
    filtered_source.push 1
    filtered_source.push 2
    filtered_source.push 3
    filtered = filtered_source.filter (% 2 == 0)

    mut retained_source = Vec::new!
    retained_source.push 1
    retained_source.push 2
    retained_source.push 3
    retained_source.retain (% 2 == 0)

    mut optional_source = Vec::new!
    optional_source.push -1
    optional_source.push 2
    optional_source.push 3
    positives = optional_source.filter_map value ->
        if value > 0
        then Option::Some value
        else Option::None

    mut checked_source = Vec::new!
    checked_source.push 4
    checked_source.push 5
    checked = checked_source.try_map value ->
        if value >= 0
        then Result::Ok value
        else Result::Err 1

    mapped.println!
    references.println!
    filtered.println!
    retained_source.println!
    positives.println!
    checked.unwrap_or Vec::new! .println!
```

The first `map` moves `source`, while `map_ref` leaves `borrowed_source` available after the callback. The output is `4`, `5`, `[2, 3, 4]`, `[5, 6]`, `[2]`, `[2]`, `[2, 3]`, and `[4, 5]`; the first two lines come from `for_each`. `try_map` returns `Ok` here. If a callback returns `Err 1`, later source elements are not mapped and the original source is consumed.

`map_ref` and `retain` pass shared references to their callbacks. Their operator sections still work: `(+ 1)` means `value -> value + 1`, and `(% 2 == 0)` means `value -> value % 2 == 0`. The integer `+` and `%` implementations use shared receivers, and method lookup adjusts the borrowed left operand to select them. No explicit `*value` is needed; the parameter remains `&I64`. See [operators on borrowed values](../language/operators.md#operators-on-borrowed-values).

The concrete methods above are the supported everyday `Vec` surface. Higher-kinded traversal interfaces are still evolving and are intentionally not expanded here. `Vec` has no `Applicative` or `Monad` implementation: choosing Cartesian application would require cloning, while choosing zipped application would not have list-monad semantics.

## `HashMap K, V`

`HashMap K, V` requires `Hash` and `Eq` for keys. The current map supports construction, length, insertion, checked lookup, and key containment.

```rock
main = !->
    mut scores = HashMap::new!
    scores.insert 10, 100
    scores.insert 20, 200
    ten = 10
    twenty = 20
    thirty = 30
    scores.contains_key &ten .println!
    scores.contains_key &thirty .println!
    match scores.get &ten
        Option::Some score => score.println!
        Option::None => -1 .println!
    match scores.get &twenty
        Option::Some score => score.println!
        Option::None => -1 .println!
    scores.len!.println!
```

The output is `true`, `false`, `100`, `200`, and `2`. `ten`, `twenty`, and `thirty` are borrowed as probe keys, so the caller retains ownership. The implementation uses open addressing and grows near a 75 percent load factor. Removal, iteration, entry APIs, and configurable hashers are not currently public.

## `Box T`

`Box::new` moves one value into a single owned heap allocation. `as_ref!` borrows it, `as_mut!` permits mutation through a mutable binding, and `Deref` forwards `*` access. Boxing is useful for stable indirection or recursive ownership; it is not a default replacement for a local value.

```rock
struct Point
    < x: I64
    < y: I64

main = !->
    point = Point
        x: 3
        y: 4
    mut boxed = Box::new point
    mutable_view = boxed.as_mut!
    mutable_view.x = 5
    view = boxed.as_ref!
    view.x.println!
    view.y.println!
    (*boxed).x.println!
```

The output is `5`, `4`, and `5`. `point` is moved into `boxed`; `as_mut!` changes the owned value through an exclusive borrow, `as_ref!` creates a shared view, and `*boxed` uses `Deref`. The box drops its value and allocation when `boxed` leaves scope.

## `Arc T`

`Arc::new` creates atomic shared ownership. `clone!` increments the reference count, but `Arc` does not make the contained value mutable. Combine it with `Mutex T` when multiple owners must update shared state.

```rock
main = !->
    state = Arc::new String::from "shared"
    worker_copy = state.clone!
    *state .println!
    *worker_copy .println!
```

The output is `shared` twice. Both `Arc` values point at the same owned string; dropping the final owner releases the allocation. A mutable `String` cannot be obtained from an `Arc String` without a separate synchronization design.

## Current resource limits

Regression tests cover `Vec` and `HashMap` element cleanup during destruction, replacement, and growth. Zero-sized `Vec` elements and `HashMap` keys or values are still rejected on insertion. Treat the prototype's resource behavior as an explicit current limit, not as a production boundary for long-running memory-heavy programs.

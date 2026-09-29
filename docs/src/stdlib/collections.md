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

Both string types contain valid UTF-8. `Char` is a 32-bit Unicode scalar value, and converting it to `String` encodes one to four UTF-8 bytes. Lengths and search offsets count bytes, not Unicode scalars or displayed characters. Strings can contain embedded NUL bytes; their stored length includes these bytes, and `println!` prints the whole string. C functions that consume null-terminated strings still stop at the first NUL.

## UTF-8 range indexing

Use a borrowed range to select text from either `&Str` or `String`:

```rock
main = !->
    text = "aé中🦀z"
    owned = String::from text
    (&text[1..3]).println!
    (&owned[3..6]).println!
    (&text[6..=9]).println!
    (&owned[..1]).println!
    (&text[10..]).println!
    (&owned[..]).println!
    string_len (&text[11..11]) .println!
```

The output is `é`, `中`, `🦀`, `a`, `z`, `aé中🦀z`, and `0`. `start..end` excludes `end`; `start..=end` includes that byte, so the checked end boundary is `end + 1`. The forms `..end`, `..=end`, `start..`, and `..` work too. Empty slices at valid boundaries, including the end of a string, are allowed.

The operation borrows the original text without allocating. The owner must remain alive while the slice is used. Negative offsets, reversed or out-of-bounds ranges, and endpoints inside a multi-byte encoding terminate the program with an error, just as invalid collection indexes do. For example, byte offset `2` in the text above is inside `é`; it is not a valid slice boundary. `str_is_char_boundary text, offset` checks a byte boundary without terminating.

String indexing uses the standard library's `Index Range` implementations. There is no integer `Index` or mutable `IndexMut` implementation for strings: arbitrary byte mutation could invalidate UTF-8. Borrow bytes explicitly when byte-level inspection is intended.

## Unicode scalar access

`str_char_at` and `String::char_at` return `Option Char` using a zero-based scalar index. A negative index or an index past the last scalar returns `None`.

```rock
main = !->
    text = "aé中🦀"
    owned = String::from text
    str_char_at text, 1 .println!
    owned.char_at 3 .println!
    owned.char_at 4 .println!
    String::from '中' .len!.println!
    bytes = owned.as_bytes!
    bytes[1].println!
    str_is_char_boundary text, 2 .println!
```

This prints `Some(é)`, `Some(🦀)`, `None`, `3`, `195`, and `false`. Scalar lookup scans from the beginning, so its cost grows with the requested index. Byte-range slicing and boundary checks take constant time. Integer-to-`Char` casts accept only Unicode scalars (`0..=0x10FFFF`, excluding surrogates `0xD800..=0xDFFF`) and trap for invalid values before narrowing.

A scalar is not necessarily a whole displayed character: a letter followed by a combining accent consists of two scalars. These APIs do not perform grapheme segmentation, normalization, or locale-sensitive comparison.

## Iterating over UTF-8 text

Both `&Str` and `String` provide `chars!` and `char_indices!`. They create small borrowing cursors without allocating a collection. `chars!` yields `Char` values; `char_indices!` yields `(I64, Char)` pairs whose first component is the character's starting byte offset.

```rock
main = !->
    text = String::from "aé🦀"
    for_each text.chars!, (!.println!)
    for (offset, ch) in text.char_indices!
        offset.println!
        ch.println!
    text.println!
```

The first traversal prints `a`, `é`, and `🦀`. The second prints offsets `0`, `1`, and `3`, each followed by its character. The final line prints `aé🦀`: traversal borrowed the owner rather than consuming it. Keep the owner alive while a cursor exists.

Each call creates a fresh cursor. Advancing through the entire text takes linear time in its byte length, and `break` stops decoding immediately. To advance manually, use `next!` on a mutable cursor; it returns `Option Char` or `Option (I64, Char)` and remains exhausted after reaching the end:

```rock
main = !->
    mut cursor = "é🦀".chars!
    cursor.next!.println!
    for ch in cursor
        ch.println!
    mut empty = "".chars!
    empty.next!.println!
```

This prints `Some(é)`, `🦀`, and `None`. The loop consumes the remaining cursor state, starting after `é`. `Chars` and `CharIndices` implement `FoldableValue`, so manual advancement, folding, `for_each`, and `for..in` share the same UTF-8 decoder. As with `char_at`, the items are Unicode scalars, not grapheme clusters.

## Validating bytes

Use `String::from_utf8` for input that might not be UTF-8. It returns `Some String` after validation and copying, or `None` for invalid bytes. Validation rejects incomplete sequences, unexpected continuation bytes, overlong encodings, surrogates, and values beyond U+10FFFF.

```rock
main = !->
    valid = [195 as U8, 169 as U8]
    invalid = [255 as U8]
    String::from_utf8 (&valid) .println!
    String::from_utf8 (&invalid) .println!
    is_utf8 (&invalid) .println!
```

This prints `Some(é)`, `None`, and `false`. `is_utf8` validates without allocating. `String::from` also accepts byte slices but terminates with `invalid UTF-8` when validation fails; prefer `from_utf8` for recoverable input errors. The unsafe `String::from_c_str` requires a readable null-terminated allocation and also validates its bytes. Raw borrowed-string construction in unsafe code must uphold UTF-8 validity and the lifetime of its backing allocation.

## Byte and search helpers

The search helpers live in `stdlib::string` and are re-exported by the prelude. Explicit imports also work; this example keeps the byte buffer separate from the borrowed `&Str` search input.

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

The output is `4`, `1`, `1`, `2`, and `111`. `string_find` returns a byte offset or `-1`; `string_contains` returns `1` or `0`; `byte_substr` returns an owned `Vec U8`; and `byte_at` reads one byte from a byte slice. Both byte helpers check bounds and terminate on an invalid index or range. They operate on arbitrary bytes and do not enforce Unicode boundaries.

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

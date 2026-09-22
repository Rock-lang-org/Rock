# References and Borrowing

A reference is a temporary access path to an existing value. `&T` is shared, while `&mut T` is exclusive and mutable. References never own the storage they point to.

## Shared References

Create a shared borrow with `&`. Any number of compatible shared borrows can coexist, and a shared receiver cannot mutate or consume the referent while that borrow is live.

```rock
length: &String -> I64
length = text -> text.len!

main = !->
    name = String::from_str "Rock"
    first = length &name
    second = length &name
    first.println!
    second.println!
    name.println!
```

Before either call, `name` owns the allocation. Each `&name` creates a shared view for one call and `length` only reads it. After both calls, `name` is still available; the output is `4`, `4`, and `Rock`.

A shared reference can be copied as a reference, but every alias keeps the borrow live until its last use.

```rock
main = !->
    mut number = 1
    first = &number
    second = first
    *first .println!
    *second .println!
    number = 2
    number.println!
```

The aliases are shared references to the same `number`. Once both aliases are last used, the borrow ends and the owner can be assigned. The output is `1`, `1`, and `2`.

## Mutable References

Create an exclusive borrow with `&mut`. The owner binding must be mutable, and no shared or second mutable borrow may overlap it.

```rock
increment: &mut I64 -> ()
increment = value !->
    *value = *value + 1

main = !->
    mut count = 4
    increment &mut count
    count.println!
```

Before the call, `count` owns `4`. During the call, `increment` has the only mutable access and writes `5` through `*value`. After the call, the borrow ends and `count` remains the owner. The output is `5`.

The common mistakes are taking `&mut` from an immutable binding, assigning to the owner while the mutable reference is still used, or creating a second overlapping mutable reference. The compiler reports these as mutability or borrow conflicts.

## Dereferencing

`*reference` accesses the referenced place. A dereference in a value position reads the referent; a dereference on the left side of assignment writes it.

```rock
read: &I64 -> I64
read = value -> *value

main = !->
    number = 17
    borrowed = &number
    result = read borrowed
    result.println!
```

`borrowed` has type `&I64`, while `result` has type `I64`. The call passes `borrowed` to `read`, whose `*value` expression copies the integer from `number` because `I64` is copyable. The output is `17`.

Method lookup performs receiver borrowing and dereferencing for both inherent methods and trait methods. Write the simplest receiver expression first; do not add `&*` repeatedly unless the signature requires an explicit reference.

A shared receiver method demonstrates automatic borrowing:

```rock
struct Counter
    < value: I64

impl Counter
    @read: I64
    @read = -> @value

main = !->
    counter = Counter
        value: 12
    result = counter.read!
    result.println!
```

`read` requires a shared receiver, but the call is written `counter.read!`, not `(&counter).read!`; method selection creates the shared borrow. Wrapper types that implement `Deref` use the same selection process to expose target methods. Keep an explicit `&value` in ordinary function calls, because automatic receiver borrowing applies to method selection rather than every argument position.

## Reborrowing

A mutable reference can be temporarily reborrowed. The shorter reborrow must finish before the original mutable reference is used again.

```rock
show_and_set: &mut I64 -> ()
show_and_set = value !->
    *value = 2
    *value .println!

main = !->
    mut value = 1
    original = &mut value
    temporary = &mut *original
    show_and_set temporary
```

`original` first owns the exclusive access. `temporary` temporarily borrows through it and is moved into `show_and_set`, so neither the original reference nor the owner is used while the reborrow is active. The output is `2`. Current borrow checking is conservative about using the original reference or owner while a named reborrow remains live; pass the reborrow to a short operation and end it before any later owner use.

## Borrow Lifetimes

A reference cannot outlive its referent. Rock ends a borrow after its final use when control flow proves that the access is finished; it does not require every borrow to last until the textual end of the block.

```rock
main = !->
    mut number = 1
    view = &number
    *view .println!
    number = 2
    number.println!
```

The shared borrow is last used by the first print. The assignment is therefore valid, and the output is `1` followed by `2`.

References to call temporaries are not valid because the temporary is destroyed before the reference could be safely used. Return a value, borrow a caller-owned value, or bind an owned result before taking a reference.

## References in Patterns

Matching through a shared reference borrows enum payloads rather than moving them. This lets a caller inspect an owned `Option String` repeatedly.

```rock
show_option: &Option String -> I64
show_option = option ->
    match *option
        Option::Some text => text.len!
        Option::None => 0

main = !->
    value = Option::Some String::from_str "hello"
    first = show_option &value
    second = show_option &value
    first.println!
    second.println!
```

The output is `5` and `5`. In each `Some` arm, `text` is a shared reference to the payload, so the `String` remains owned by `value`.

## Receiver Modes

Method receiver markers are reference contracts as well as ownership contracts.

```rock
struct Counter
    < value: I64

impl Counter
    @get: I64
    @get = -> @value

    ^@increment: ()
    ^@increment = !->
        self.value = self.value + 1

    ~@finish: I64
    ~@finish = -> self.value

main = !->
    mut counter = Counter
        value: 1
    counter.get!.println!
    counter.increment!
    counter.get!.println!
    counter.finish!.println!
```

`get!` creates shared access, `increment!` creates exclusive mutable access, and `finish!` moves the receiver. The output is `1`, `2`, and `2`; the binding `counter` is unavailable after `finish!`.

## Current Limitations

Bare `[T]` and bare `Str` are not ordinary standalone parameter or field types; use `&[T]`, `&mut [T]`, or `&Str`. Mutable references are not copyable. References to temporaries are rejected, and a shared borrow of a non-copy owned value cannot be dereferenced to move that value out. Raw pointers are a separate unsafe interface and are not needed for ordinary borrowing.

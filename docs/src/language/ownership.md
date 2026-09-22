# Ownership and Moves

Ownership answers one question for every value: which binding is responsible for using and eventually releasing it? Rock uses that information to make moves, borrows, and cleanup explicit without a garbage collector.

## Copyable Values

The compiler knows that primitive values such as `I64` and `Bool` can be copied. Assignment creates another independent value, so the original remains usable.

```rock
main = !->
    first = 42
    second = first
    first.println!
    second.println!
```

Both bindings contain `42`, and the output is two lines containing `42`. `first` and `second` do not share an owned allocation.

Rock does not expose a user-defined `Copy` trait. Repeat-array literals and other operations that duplicate a value use compiler-known copy facts. Do not assume that a generic `T` is copyable merely because a concrete call later happens to use `I64`.

## Moving Owned Values

Heap-backed values such as `String` and `Vec T` own resources. Passing one by value transfers responsibility to the parameter or destination. The source binding cannot be used again after the move.

```rock
string_length: String -> I64
string_length = text -> text.len!

main = !->
    name = String::from_str "Rock"
    length = string_length name
    length.println!
```

Before the call, `name` owns the string allocation. The call moves that `String` into `string_length`, whose parameter owns it while the function runs. After the call, `name` is moved and is not referenced again; the returned `length` is an independent `I64`. The output is `4`.

The common mistake is to use the source after the call. If the caller needs the value afterward, pass a reference or clone it explicitly rather than pretending the move did not happen.

## Borrowing Instead of Moving

A reference lets a function inspect an owned value without becoming its owner. Put the reference in the signature so callers can see the ownership contract.

```rock
measure: &String -> I64
measure = text -> text.len!

main = !->
    name = String::from_str "Rock"
    length = measure &name
    length.println!
    name.println!
```

Before the call, `name` owns the allocation. `&name` creates a shared borrow, and `measure` reads through it. After the borrow's last use, `name` is still the owner and can be printed. The output is `4` and `Rock`.

Use a borrowed parameter when the operation only observes data. Use a by-value parameter when the operation must forward or consume the resource.

## Consuming Standard-Library Operations

Many transformations consume an owned receiver. `Option::map` can move an owned payload into its callback, and `Vec::map` can move every owned element into a new vector.

```rock
string_length: String -> I64
string_length = text -> text.len!

main = !->
    mut words = Vec::new!
    words.push String::from_str "a"
    words.push String::from_str "long"
    lengths = words.map string_length
    lengths[0].println!
    lengths[1].println!
```

Before `map`, `words` owns both strings. `words.map string_length` consumes `words`; each `String` moves into `string_length`, and the new `lengths` owns the two `I64` results. The output is `1` and `4`. The old `words` binding is not used afterward.

## Receiver Ownership

Methods state the same contract with a marker:

- `@method` borrows the receiver shared.
- `^@method` borrows the receiver mutably.
- `~@method` moves the receiver.

```rock
struct Counter
    < value: I64

impl Counter
    @read: I64
    @read = -> @value

    ^@set: I64 -> ()
    ^@set = value !->
        self.value = value

    ~@take: I64
    ~@take = -> self.value

main = !->
    mut counter = Counter
        value: 1
    counter.read!.println!
    counter.set 9
    counter.read!.println!
    counter.take!.println!
```

`read!` leaves `counter` available because it borrows. `set 9` requires a mutable binding and changes the owned value in place. `take!` consumes `counter`; the final `I64` is returned, and `counter` is not used again. The output is `1`, `9`, and `9`.

## Explicit Cloning

`Clone` requests a second owned value. It is different from compiler-known copying and may allocate or recursively clone fields.

```rock
main = !->
    first = String::from_str "owned"
    second = first.clone!
    first.println!
    second.println!
```

`first.clone!` borrows `first`, allocates a second string, and returns it. Both bindings remain usable, so the output contains `owned` twice. Clone only when two owners are actually required; a shared borrow is cheaper when the second owner is unnecessary.

## Scope and Cleanup

An owned value is cleaned up when its live scope ends. `String`, `Vec`, and `Box` use standard-library ownership and `Drop` implementations for this purpose.

```rock
make_message: () -> String
make_message = -> String::from_str "temporary"

main = !->
    make_message!.len!.println!
```

`make_message!` returns ownership of a `String` to `main`. `main` owns it until the function ends, after printing `9`. There is no implicit exception unwinding in Rock; errors are ordinary `Result` values and cleanup follows explicit control flow.

The `Drop` trait gives an owned type a cleanup method. Cleanup is automatic at scope end; calling the method explicitly is not required.

```rock
struct Resource
    < id: I64

impl Drop for Resource
    ~@drop = -> return

main = !->
    resource = Resource
        id: 7
    resource.id.println!
```

`resource` owns a `Resource` until `main` ends, when its `Drop` implementation runs. The output is `7`.

## Partial Moves and Current Limits

The checker tracks moves through fields, tuple elements, enum payloads, branches, and loops. Moving one non-copy field can leave the rest of an aggregate initialized, but direct cleanup types are intentionally conservative: moving part of a value that owns cleanup may be rejected rather than risk double cleanup. Destructure deliberately or borrow a field when the whole owner must remain usable.

Here, moving `first` does not prevent using the independent `second` field:

```rock
struct Names
    < first: String
    < second: String

take_length: String -> I64
take_length = text -> text.len!

main = !->
    names = Names
        first: String::from_str "Ada"
        second: String::from_str "Lovelace"

    take_length names.first .println!
    names.second.len! .println!
```

`take_length names.first` moves only the first `String`. The second field remains initialized and prints length `8`; trying to use `names.first` or move the complete `names` value afterward would be rejected. If `Names` itself implemented `Drop`, moving out one field could be rejected because cleanup would otherwise receive a partially initialized owner.

The practical rules are simple: after a by-value call, consider the argument moved; after a shared call, the owner remains available; after a mutable or consuming method, follow its marker; and use `clone!` when independent ownership is required.

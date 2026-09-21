# Unsafe Rock and Raw Pointers

Safe Rock checks ownership, initialization, and borrowing. `unsafe` marks a
small boundary where the programmer accepts an invariant the compiler cannot
prove. It does not disable ordinary type checking or name resolution.

## Raw pointer types

`*T` is a raw pointer type. A pointer value must have a documented source; an
invented integer or null value is not a valid target. The type and dereference
grammar are distinct:

```text
POINTER_TYPE ::= "*" TYPE
POINTER_READ ::= "unsafe" "*" EXPRESSION
POINTER_WRITE ::= "unsafe" "*" EXPRESSION "=" EXPRESSION
```

This complete program obtains a pointer from a live mutable borrow and writes
through it while the owner remains alive:

```rock
unsafe set_i64: &mut I64 -> ()
set_i64 = reference ->
    pointer: *I64 = reference as *I64
    unsafe *pointer = 7
    return

main = !->
    mut value: I64 = 42
    unsafe set_i64 &mut value
    value.println!
```

The contract for `set_i64` is that the argument is aligned and initialized for
`I64`, remains live for the call, and is not accessed through another
incompatible alias during the write. The pointer is borrowed, not owned, so
the function must not return it or call a deallocator.

## Unsafe functions

Mark a function unsafe when every caller must uphold a contract. The unsafe
declaration appears before its definition and before its call:

```rock
unsafe read_i64: &I64 -> I64
read_i64 = reference ->
    pointer: *I64 = reference as *I64
    unsafe *pointer

main = !->
    value: I64 = 8
    result: I64 = unsafe read_i64 &value
    result.println!
```

The contract states validity, alignment, initialization, aliasing, and lifetime
in prose next to the function. Calling the function without `unsafe` is a
compiler error; the marker makes the review boundary visible at every call
site.

## Pointer casts

`as` changes the pointer type view. It does not allocate, initialize, change
alignment, or extend a lifetime. This complete example casts a mutable borrow
only to immediately write the same `I64` representation:

```rock
unsafe increment_i64: &mut I64 -> ()
increment_i64 = reference ->
    pointer: *I64 = reference as *I64
    current: I64 = unsafe *pointer
    unsafe *pointer = current + 1
    return

main = !->
    mut value: I64 = 4
    unsafe increment_i64 &mut value
    value.println!
```

Do not cast a pointer merely to bypass a failed borrow. A cast is justified
only when the target representation and all aliasing rules have already been
established by the wrapper's contract.

## Owning foreign allocations

An owning pointer needs one allocator and one matching release path. The
following complete program uses the POSIX stdlib declarations, checks the
allocation result, never dereferences uninitialized memory, and frees the
allocation once:

```rock
> stdlib::libc::malloc
> stdlib::libc::free

unsafe allocate_and_release: I64 -> I64
allocate_and_release = size ->
    if size <= 0
        -1
    else
        pointer: *U8 = malloc size
        if (pointer as I64) == 0
            -1
        else
            free pointer
            size

main = !->
    result: I64 = unsafe allocate_and_release 8
    result.println!
```

Its contract is that positive `size` is a byte count, `malloc` and `free` are
the matching C allocator pair, and no caller retains `pointer` after the
wrapper returns. A production owner would store the pointer and size together,
define cleanup for every early-return path, and expose initialized operations
rather than a raw pointer. It must not pass a Rock-owned allocation to `free`.

## Building a safe abstraction

Keep unsafe code small and put the invariant at its edge:

1. State the representation, alignment, initialization, aliasing, lifetime,
   and ownership contract.
2. Check every condition that safe code can check, including lengths and null
   results.
3. Perform the minimum raw operation.
4. Return a value or safe owner whose public methods preserve the invariant.
5. Release each owned allocation exactly once, using its creating allocator.

`String`, `Vec`, `Box`, `Arc`, files, sockets, and synchronization primitives
follow this general shape, although the prototype still has known cleanup gaps.

## What unsafe cannot justify

An unsafe block cannot make a dangling pointer valid, make a data race safe,
initialize bytes that were never written, or give two owners permission to
free one allocation. It also cannot make a foreign ABI compatible. If the
invariant cannot be explained locally, redesign the boundary or return a
structured error before entering unsafe code.

## Current target caveat

The current FFI-backed implementations assume POSIX behavior and the
`x86_64-unknown-linux-gnu` target. Raw pointer widths, C integer conventions,
linker libraries, and synchronization ABIs can differ elsewhere. Keep
target-specific declarations isolated and compile each unsafe boundary on the
target where it will run.

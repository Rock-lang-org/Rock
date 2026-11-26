# Foreign Functions

`extern` declares a function whose implementation is supplied by the linker,
usually by a C-compatible library. The declaration is a type contract, not an
implementation or a request to search the host for an arbitrary symbol.

## A scalar function

This complete program declares the foreign symbol before using it:

```rock
extern sqrt: F64 -> F64

main = !->
    result: F64 = sqrt 144.0
    result.println!
```

The target C library must export `sqrt` with the declared calling convention.
`F64` is a fixed-width floating representation suitable for this simple
boundary. The program owns no memory passed to `sqrt`, so there is no cleanup
operation associated with this call.

Build this kind of example with `rock` only on the documented target,
`x86_64-unknown-linux-gnu`. A different target may use a different library
name, ABI, or linker setup.

## Foreign names that are not Rock names

Rock function names start with a lowercase letter, so a C symbol such as
`GetTickCount` or `CreateThread` cannot be written as an ordinary `extern`
name. Put the exact symbol in quotes after `extern` and choose a Rock name for
it. This complete program calls C's `abs` under the Rock name `c_abs`:

```rock
extern "abs" c_abs: I32 -> I32

main = !->
    result: I32 = c_abs (-7 as I32)
    result.println!
```

The program prints `7`. Only the linker sees `abs`; Rock code always calls
`c_abs`. The quoted symbol is used exactly as written, so it must match the
library's exported name, including case.

## Pointer contracts

Raw pointers do not prove validity, alignment, initialization, aliasing, or
lifetime. The contract belongs next to the declaration that accepts one. This
complete example converts a live mutable borrow into a raw pointer, reads the
same initialized `I64`, and never transfers ownership:

```rock
unsafe read_i64: &mut I64 -> I64
read_i64 = reference ->
    pointer: *I64 = reference as *I64
    unsafe *pointer

main = !->
    mut value: I64 = 41
    result: I64 = unsafe read_i64 &mut value
    result.println!
```

The caller contract is: `reference` points to the live `value` for the whole
call, is aligned for `I64`, is initialized, and is not accessed through an
incompatible alias while the call runs. The function returns before the
borrowed storage can end, so it returns an integer rather than the pointer.
There is no `free` call because the pointer is a borrowed view of a stack local.

Never invent an address or use a null pointer as a test value. A pointer must
come from a live borrow, a documented allocator, or a foreign API that has
returned ownership under an explicit contract.

## C strings and borrowed ownership

The standard library exports the POSIX `strlen` declaration from a non-prelude
module. This complete wrapper imports it explicitly, declares its pointer local,
and keeps the owning `String` alive until the foreign call returns:

```rock
> std::ffi::libc::strlen

unsafe c_string_length: String -> I64
c_string_length = text ->
    pointer: *U8 = text.as_ptr!
    strlen pointer

main = !->
    text: String = String::from "Rock"
    length: I64 = unsafe c_string_length text
    length.println!
```

The imported declaration is equivalent to `extern strlen: *U8 -> I64` in the
std's libc module. Its contract requires a non-null pointer to a readable,
initialized, null-terminated byte sequence. `String` supplies that terminator;
the wrapper moves the owner into the function and drops it only after `strlen`
has returned. A borrowed `&Str` is not automatically safe for this API because
it may be length-delimited and may contain an interior null byte.

## Allocation and release

When a foreign allocator returns ownership, pair it with the matching foreign
release function. This complete example never dereferences the allocated
bytes, rejects non-positive sizes, checks the returned pointer, and frees one
successful allocation exactly once:

```rock
> std::ffi::libc::malloc
> std::ffi::libc::free

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
    result: I64 = unsafe allocate_and_release 16
    result.println!
```

The `malloc` and `free` declarations are exported by the POSIX-oriented std
libc module. The wrapper's unsafe contract is that `size` is a positive byte
count and that the returned pointer is freed only by the allocator that created
it. Since the example does not write or read the allocation, it does not claim
an initialized element layout. A real owner type must also define what happens
on every construction failure and must release the pointer exactly once.

## ABI-compatible types

Prefer fixed-width primitives such as `I32`, `I64`, `U32`, `U64`, and `F64` at a
foreign boundary. Use raw pointers only with a written representation and
lifetime contract. Do not assume that `Bool`, an enum, a generic struct, a
closure, an ordinary function value, or a Rock struct has the C ABI layout you
want; Rock does not yet provide stable C-layout attributes.

Every pointer crossing the boundary needs these answers:

- Which side allocated it?
- Which side releases it, and with which allocator?
- Is it borrowed, and until when?
- May the foreign function retain it after the call?
- Can a callback run after the Rock stack frame returns?
- How are errors and sentinel values represented?

Never free a Rock-owned pointer through an unrelated C allocator. Never return
a pointer into a local or an already-dropped `String`. Keep the unsafe wrapper
small and expose a safe Rock API that preserves the representation and cleanup
invariant.

## Target scope

The shipped FFI-backed std assumes POSIX APIs. Its operating-system details
(errno access, `open` flags, socket address layout) live in a private backend
selected with [`cfg` blocks](../programs/modules.md#target-specific-modules-with-cfg);
Linux, macOS (POSIX) and Windows (C runtime, Winsock and Win32 threads) each
have a backend. All three pass the std platform smoke program on native
machines, but only Linux runs the full test suite and ships release binaries. Raw `std::ffi::libc` bindings whose C signature differs between
targets, such as the POSIX socket and thread functions, `open`, `atol` and
`__errno_location`, exist only on the targets where their signature is known. Put your own foreign declarations
in `cfg`-selected modules the same way, and isolate them behind portable APIs. A
program that needs nonblocking sockets or a C-layout aggregate must provide a
separate boundary implementation and test each target explicitly.

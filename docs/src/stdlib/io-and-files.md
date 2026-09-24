# Input, Output, and Files

Rock's I/O abstractions use `Result` and ownership rather than exceptions or manual close calls. `File` and `TcpStream` implement `Read` and `Write`, `Stdin` implements `Read`, and `Stdout` implements `Write`. Generic code can therefore work with more than one kind of byte stream.

## The `Read` and `Write` traits

`Read` takes a mutable byte slice and reports a byte count or `IoError`. `Write` borrows the output handle and the source data. Import the trait when calling its methods through a generic bound or a concrete value.

| Method | Contract |
| --- | --- |
| `reader.read buffer` | Reads up to the buffer length; a short read is successful. |
| `writer.write bytes` | Attempts one write; success may report fewer bytes than supplied. |
| `writer.write_all bytes` | Repeats writes until all bytes are written or an error occurs. |
| `writer.write_str text` | Writes the entire string's bytes using the complete-write helper. |

```rock
> stdlib::fs::File
> stdlib::io::IoError
> stdlib::io::Read
> stdlib::io::Write

write_text: &W -> &Str -> Result I64, IoError where W: Write
write_text = writer, text ->
    writer.write_str text

read_text_prefix: &mut R -> &mut [U8] -> Result I64, IoError where R: Read
read_text_prefix = reader, buffer ->
    reader.read buffer

write_demo: &Str -> Result I64, IoError
write_demo = path ->
    file = File::create path?
    write_text &file, "hello"

main = !->
    (write_demo "rock-io-traits.txt")
        .unwrap_or -1
        .println!
```

This example writes five bytes and prints `5`; `read_text_prefix` demonstrates the reader signature even though `main` only uses the write path. `read` needs exclusive access to both the reader and buffer. Writing needs only a shared handle, so the file does not need a mutable binding. The file closes automatically when its owner is dropped.

The free functions `stdlib::io::write_all` and `stdlib::io::write_str` supply the same complete-write behavior for any `W: Write`; custom implementations can delegate their helper methods to them. If a write makes no progress while bytes remain, the helper returns `IoError::WriteZero` rather than looping forever. Other errors propagate immediately, including interrupted operating-system calls. An error may occur after some bytes have already been written; it does not undo those bytes, and the error result does not carry the partial count.

## Files

`File::open` reads an existing path, `File::create` creates or truncates a path, and `File::append` creates or appends. Each returns `Result File, IoError`, so a function that uses `?` must return a compatible `Result`.

```rock
> stdlib::fs::File
> stdlib::io::IoError
> stdlib::io::Write

write_greeting: &Str -> Result I64, IoError
write_greeting = path ->
    file = File::create path?
    file.write_str "Hello from Rock\n"

append_greeting: &Str -> Result I64, IoError
append_greeting = path ->
    file = File::append path?
    file.write_str "Again\n"

main = ->
    result = do
        count <- write_greeting "rock-greeting.txt"
        appended <- append_greeting "rock-greeting.txt"
        pure (count + appended)
    status = result <&> total ->
        total.println!
        0
    status.unwrap_or 1
```

The successful output is `22`, and `rock-greeting.txt` contains `Hello from Rock` followed by `Again`, each followed by a newline. The descriptor is closed by `Drop`; callers do not call a separate close API. `create` and `append` can create a missing file, as this example requires; `open` reports an operating-system `IoError::Os code` when the file is absent. A path containing a null byte reports `IoError::InvalidPath` before the operating-system call.

`do` keeps the successful counts in scope without nested callbacks. Each `<-` sequences through the prelude's `bind`, which continues only for `Result::Ok`; `pure` wraps the sum in the same carrier. This has the sequencing behavior of `>>=`. At the boundary, `<&>` prints a successful total and produces exit code `0`, while `unwrap_or 1` turns either operation's error into exit code `1`. See [error handling](../functional/error-handling.md) for `do` and the named combinators.

## Reading bytes

Only the prefix indicated by the returned count contains newly read bytes. A zero count from a read into a nonempty buffer means end of file. A read is not a request to fill the buffer: loop until the format's expected length or end of file. This complete program creates its input first, then copies it to standard output using a small buffer.

```rock
> stdlib::fs::File
> stdlib::io::IoError
> stdlib::io::Read
> stdlib::io::Write
> stdlib::io::stdout

prepare: &Str -> Result I64, IoError
prepare = path ->
    file = File::create path?
    file.write_str "world"

read_file: &Str -> Result I64, IoError
read_file = path ->
    mut file = File::open path?
    output = stdout!
    mut bytes: [U8; 3] = [0; 3]
    mut total = 0
    mut count = file.read (&mut bytes)?
    while count > 0
        written = output.write_all (&bytes[..count])?
        total = total + written
        count = file.read (&mut bytes)?
    Result::Ok total

main = ->
    copied = do
        prepare "rock-io-input.txt"
        read_file "rock-io-input.txt"
    status = copied <&> count -> if count == 5 then 0 else 1
    status.unwrap_or 1
```

The output is `world`, without a newline. The caller checks that five bytes were copied before reporting success. Each `&bytes[..count]` borrows just the initialized prefix from this read, not any leftover bytes from the previous iteration. The shared borrow is finished before the next mutable read. The file owns its descriptor throughout and closes when `read_file` returns.

### Prefixes, Subslices, and String Bytes

Select the data first, then write the resulting slice. Prefix and subslice writes need neither a raw pointer nor a separate length parameter:

```rock
> stdlib::io::IoError
> stdlib::io::Write
> stdlib::io::stdout
> stdlib::string::str_as_bytes

write_parts: () -> Result I64, IoError
write_parts = ->
    output = stdout!
    bytes = str_as_bytes "hello"
    first = output.write_all (&bytes[..2])?
    rest = output.write_all (&bytes[2..])?
    Result::Ok (first + rest)

main = ->
    status = write_parts! <&> _ -> 0
    status.unwrap_or 1
```

This writes `hello`. `str_as_bytes` borrows an `&Str` as `&[U8]`; an owned `String` offers the equivalent `as_bytes!` method. Neither operation copies the data. The offsets count bytes, not Unicode characters, and the resulting view is a byte slice, not a string. Range bounds are checked and invalid bounds terminate the process rather than returning `IoError`.

## Standard streams

`stdout!` and `stdin!` create lightweight handles. The output example is deterministic and avoids requiring interactive input; `Stdin` implements the same `Read` contract for a caller that supplies a mutable buffer.

```rock
> stdlib::io::stdout
> stdlib::io::Write

main = ->
    output = stdout!
    bytes: [U8; 5] = [104, 101, 108, 108, 111]
    status = output.write_all (&bytes[..3]) <&> _ -> 0
    status.unwrap_or 1
```

The process writes `hel` to standard output and returns `0`. The native range creates a borrowed three-byte view, and `write_all` handles partial operating-system writes until the whole slice is sent or an error occurs.

Reading standard input uses the same ownership contract. This program performs one read and requires input at runtime; it reports the number of bytes received, not necessarily all the bytes the producer will send.

```rock
> stdlib::io::IoError
> stdlib::io::Read
> stdlib::io::stdin

read_stdin: () -> Result I64, IoError
read_stdin = ->
    mut input = stdin!
    mut buffer: [U8; 16] = [0; 16]
    input.read &mut buffer

main = !->
    read_stdin! .unwrap_or -1 .println!
```

## Copying and the `|>>` operator

The generic `copy` function reads from `&mut R` and writes to `&W` until end of file. `|>>` is its pipeline-oriented spelling and consumes the source reader while borrowing the destination writer. This program creates a source file so the example has no hidden input.

```rock
> stdlib::fs::File
> stdlib::io::IoError
> stdlib::io::Write

prepare: &Str -> Result I64, IoError
prepare = path ->
    file = File::create path?
    file.write_str "functional byte pipe"

copy_file: &Str -> &Str -> Result I64, IoError
copy_file = source, target ->
    output = File::create target?
    (File::open source?) |>> &output

main = ->
    copied = do
        prepare "rock-copy-input.txt"
        copy_file "rock-copy-input.txt", "rock-copy-output.txt"
    status = copied <&> count ->
        count.println!
        0
    status.unwrap_or 1
```

The output is `20`, and the destination contains the same 20 bytes. The source `File` is moved into the pipe, while `output` remains the destination owner. This shape is useful for generic file, socket, and standard-stream code.

## Program arguments

`args!` is an explicitly imported function that returns an owned `Vec String` snapshot. It includes `argv[0]`, so a run with no user arguments still has length `1`.

```rock
> stdlib::env::args

main = !->
    values: Vec String = args!
    values.len!.println!
```

With no user arguments the output is `1`; with `first second` after the run separator it is `3`. The vector owns each argument string, and there is currently no standard environment-variable API.

## Errors, ownership, and portability

Use a concrete error type such as `IoError` in public signatures. Use `>>=` to sequence fallible operations, `<&>` to transform successes, and `?` to propagate failures from a helper that returns a compatible carrier. At the boundary, `unwrap_or` supplies a fallback value; use `match` when you need to inspect and report the particular error. Moving a `File` transfers descriptor ownership; borrowing it for `Read` or `Write` leaves the caller responsible for the eventual drop. A failed `?` returns early and does not restore values already moved into the failed operation.

The current file and stream implementations are POSIX-oriented. Error numbers and some behavior depend on the host operating system, and the examples that create files require a writable working directory.

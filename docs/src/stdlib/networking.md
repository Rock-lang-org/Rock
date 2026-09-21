# Networking

The current networking module provides safe ownership wrappers around blocking IPv4 TCP sockets. It exposes address values, a listener, and a stream; descriptors close through `Drop`, and all fallible operations return a `Result` carrying `IoError`.

## Addresses

`Ipv4Addr::new` accepts four octets. `SocketAddrV4::new` combines an address and an integer port. The convenience constructors `localhost` and `any` use loopback and all-interface addresses respectively.

```rock
> stdlib::net::Ipv4Addr
> stdlib::net::SocketAddrV4

main = !->
    loopback: Ipv4Addr = Ipv4Addr::new 127, 0, 0, 1
    address: SocketAddrV4 = SocketAddrV4::new loopback, 8080
    local: SocketAddrV4 = SocketAddrV4::localhost 9000
    wildcard: SocketAddrV4 = SocketAddrV4::any 7000
    address.ip.a.println!
    address.port.println!
    local.ip.d.println!
    wildcard.ip.a.println!
```

The output is `127`, `8080`, `1`, and `0`. Ports are checked to fit the valid range `0..=65535` when a listener or stream is created. A port of `0` asks the operating system to select an available port for a listener.

## Connecting

`TcpStream::connect` returns `Result TcpStream, IoError`. This deterministic example uses an invalid port to show the library-owned validation error without depending on another process listening on a particular port.

```rock
> stdlib::io::IoError
> stdlib::net::Ipv4Addr
> stdlib::net::SocketAddrV4
> stdlib::net::TcpStream

connect_checked: () -> Result TcpStream, IoError
connect_checked = ->
    address = SocketAddrV4::localhost 70000
    TcpStream::connect address

main = ->
    match connect_checked!
        Result::Ok _ => 1
        Result::Err IoError::InvalidAddress =>
            "invalid port".println!
            0
        Result::Err _ => 2
```

The output is `invalid port`. For a valid address, `connect` performs a blocking IPv4 TCP connection and can return an operating-system error when no server is listening. The returned stream owns its descriptor; move it to the component that owns the connection and do not access it after that move.

## Listening and accepting

`TcpListener::bind` creates a blocking listener, `local_addr!` reports the selected address, and `accept!` waits for a client and returns a new owned `TcpStream`. Binding port zero makes the setup reproducible without reserving a fixed port.

```rock
> stdlib::io::IoError
> stdlib::net::SocketAddrV4
> stdlib::net::TcpListener

open_listener: () -> Result SocketAddrV4, IoError
open_listener = ->
    listener = TcpListener::bind SocketAddrV4::localhost 0?
    listener.local_addr!

main = ->
    match open_listener!
        Result::Ok address =>
            address.port > 0 .println!
            address.ip.a as I64 .println!
            0
        Result::Err _ => 1
```

The output is `true` and `127`. A listener created with port zero must be queried with `local_addr!` before a client can use the selected port. `accept!` is intentionally omitted from this setup-only example because it blocks until a client connects; the complete round-trip below exercises it with an in-process client.

## Reading, writing, and shutdown

`TcpStream` implements the generic `Read` and `Write` traits and also has direct `recv`, `send`, `shutdown`, and `shutdown_write` methods. This complete round-trip writes a native subslice, half-closes the client write side, then receives the sent bytes on the server side.

```rock
> stdlib::io::IoError
> stdlib::io::Write
> stdlib::net::SocketAddrV4
> stdlib::net::TcpListener
> stdlib::net::TcpStream

roundtrip: () -> Result I64, IoError
roundtrip = ->
    listener = TcpListener::bind SocketAddrV4::localhost 0?
    address = listener.local_addr!?
    client = TcpStream::connect address?
    server = listener.accept!?
    bytes: [U8; 5] = [112, 105, 110, 103, 120]
    written = client.write_all (&bytes[..4])?
    client.shutdown_write!?
    mut received: [U8; 4] = [0; 4]
    mut total = 0
    while total < 4
        count = server.recv (&mut received[total..])?
        if count == 0
            failure: Result I64, IoError = Result::Err IoError::InvalidInput
            return failure
        total = total + count
    server.shutdown!?
    written.println!
    total.println!
    received[0] as I64 .println!
    received[3] as I64 .println!
    Result::Ok 0

main = ->
    match roundtrip!
        Result::Ok code => code
        Result::Err _ => 1
```

On success, the output is `4`, `4`, `112`, and `103`, corresponding to `ping`. `write_all` sends the four-byte prefix, while each `recv` fills only the still-empty suffix of the destination. The loop handles arbitrary short reads. This example uses `InvalidInput` to report an early end of stream because the current `IoError` has no dedicated unexpected-EOF variant.

`shutdown_write!` prevents further client writes while permitting reads; it also lets the peer observe end of stream after draining the sent bytes. `shutdown!` disables both directions. Neither operation releases descriptor ownership: the stream's `Drop` still closes the descriptor. Both shutdown calls return `Result (), IoError`, so their errors are propagated here.

Use `write_all (&buffer[..count])` to forward a received prefix and `write_all (&buffer[start..end])` to send a middle portion. The slice carries its own length; there is no separate prefix-count argument. `send`, like the trait method `write`, performs a single write and may return a short count. `write_str` writes all string bytes. `recv` takes a shared stream handle and a mutable buffer; the trait method `read` requires a mutable stream receiver as well. For a nonempty receive buffer, `Ok 0` means the peer has ended its sending direction.

## Ownership and errors

Listeners and streams are ordinary owned values. Moving an accepted stream into a worker transfers descriptor responsibility. `Arc` can share ownership, but it does not serialize complete messages: concurrent writers can interleave their writes. Use a single writer or explicit synchronization when ordering matters. `IoError::InvalidAddress` covers an invalid port, while `IoError::Os code` carries an operating-system failure such as a refused connection or failed bind. Complete-write helpers can also return `IoError::WriteZero`.

Do not read a TCP message by assuming one `recv` equals one application message. TCP is a byte stream: implement framing, handle partial reads and writes, and define a shutdown path before adding protocol logic.

## Current scope and limits

Networking is intentionally small:

- IPv4 only;
- blocking calls only;
- TCP only;
- no TLS;
- no async runtime;
- no socket-timeout abstraction.

Threads can provide limited blocking concurrency, but there are no channels or nonblocking task APIs. A production service also needs protocol framing, resource limits, backpressure, and platform-aware error handling.

For a complete request/response example, continue to [Building an HTTP Server](http.md). The separate `rock_http` package implements bounded HTTP parsing and response framing over these safe TCP APIs; it is a local dependency, not a module inside `stdlib`.

# Building an HTTP Server

TCP delivers bytes, not requests. An HTTP server must recognize where headers end, wait for the declared body length, and write a response whose framing matches its body. The `rock_http` library supplies those operations using Rock's safe I/O, collections, and threading APIs.

`rock_http` is a separate package, not part of the standard library. It lives in the [rock-lang-org/rock_http](https://github.com/rock-lang-org/rock_http) repository. Import it as `rock_http::...`, not `stdlib::http::...`.

In this chapter we will build a small loopback server with a health endpoint and a binary echo endpoint. It is a local learning project, not a hardened Internet-facing service.

Before starting, read [Option, Result, and `?`](../functional/error-handling.md), [Packages and Dependencies](../programs/packages.md), and [Input, Output, and Files](io-and-files.md). The [networking](networking.md) and [concurrency](concurrency.md) chapters explain the socket and worker ownership that the library manages for us.

## The Program's Architecture

The application and library have distinct responsibilities:

```text
TCP connection
    -> bounded request parser
    -> owned Request
    -> application handler: routing and response construction
    -> owned Response
    -> framing validation and complete writes
    -> connection closes
```

Keep routing separate from socket operations. The handler can be exercised with an owned request without accepting a connection, while the parser can be exercised with a byte buffer without starting a server. This separation also keeps partial reads, response lengths, and descriptor cleanup out of application branches.

## Create the Project

Use Rock `v0.5.2` with its matching standard library, or a compatible development toolchain. The commands below use the `rock` application CLI, which builds local dependencies and passes their artifacts to the compiler for you.

The complete project is included at `docs/examples/http-server/`. From the Rock repository root, clone the library alongside the compiler checkout and select the revision used by this example:

```console
$ git clone https://github.com/rock-lang-org/rock_http.git ../rock_http
$ git -C ../rock_http checkout --detach 55967acf9850660595f187a503c4c79661cd2479
```

The checkouts have this layout:

```text
rock_http/
  rock.toml
  src/lib.rk
Rock/
  docs/
    examples/
      http-server/
        rock.toml
        src/main.rk
```

The following listings are the two files of this project, not independent single-file programs. The application manifest, `docs/examples/http-server/rock.toml`, is:

```toml
[crate]
name = "book_http_server"
version = "0.1.0"

[lib]
path = "src/main.rk"

[dependencies]
rock_http = { path = "../../../../rock_http" }
```

The current manifest format uses `[lib].path` for the application's entry file too. Dependency paths are relative to the directory containing that manifest, not the shell's current directory. From `docs/examples/http-server`, four parent steps reach the directory containing both checkouts. If you place the application elsewhere, adjust the path to the actual `rock_http` directory.

Do not replace this path with a registry version or Git dependency: those manifest dependency sources are not implemented. The explicit checkout above pins the library revision instead. Do not add a second local `stdlib` dependency for this tutorial either; `rock` supplies the selected toolchain's standard-library artifact to both packages.

## Handle a Request

Put the following complete application in `src/main.rk`:

```rock
> stdlib::net::SocketAddrV4
> rock_http::config::ServerConfig
> rock_http::error::HttpError
> rock_http::request::Method
> rock_http::request::Request
> rock_http::response::Response
> rock_http::server::Server

handle: Request -> Response
handle = request ->
    if request.target.as_str! == "/health"
        allowed = match *(&request.method)
            Method::Get => true
            Method::Head => true
            _ => false
        if allowed
            (Response::text 200, "ok\n").with_header "Content-Type", "text/plain"
        else
            (Response::text 405, "method not allowed\n").with_header "Allow", "GET, HEAD"
    else if request.target.as_str! == "/echo"
        allowed = match *(&request.method)
            Method::Post => true
            _ => false
        if allowed
            (Response::bytes 200, request.body).with_header "Content-Type", "application/octet-stream"
        else
            (Response::text 405, "method not allowed\n").with_header "Allow", "POST"
    else
        (Response::text 404, "not found\n").with_header "Content-Type", "text/plain"

run_server: () -> Result (), HttpError
run_server = ->
    config = ServerConfig
        max_request_line_bytes: 8192
        max_header_bytes: 32768
        max_headers: 100
        max_body_bytes: 65536
    server = Server::bind_with_config (SocketAddrV4::localhost 8080), config?
    "Listening on http://127.0.0.1:8080".println!
    server.serve handle

main = ->
    match run_server!
        Result::Ok _ => 0
        Result::Err _ =>
            "HTTP server stopped with an error".println!
            1
```

`handle` consumes one owned `Request` and returns one owned `Response`. Borrowing `request.method` for the match lets us inspect the enum without consuming the request's data. For `/echo`, `Response::bytes` takes ownership of `request.body`, a `Vec U8`; there is no need to reinterpret arbitrary bytes as text or copy the body into another vector.

`Response::text` copies its `&Str` argument into an owned string. `with_header` consumes the response and returns it with another header appended, which allows the construction and header call to be chained. Text responses do not automatically receive a `Content-Type`: the handler sets `text/plain` for the successful health response and the 404 response, while the 405 responses add only an `Allow` header.

The routing here is deliberately exact. `Request.target` is the raw request target, not a parsed URL: `/health?verbose=1` does not match `/health`. There is no automatic query parsing, percent decoding, routing, JSON decoding, or content negotiation. `Method` recognizes GET, HEAD, POST, PUT, DELETE, OPTIONS, and PATCH; other valid method tokens are represented as `Method::Other String`.

## Run and Call the Server

From the repository root, enter the project and build it:

```console
$ cd docs/examples/http-server
$ rock build
$ rock run
Listening on http://127.0.0.1:8080
```

`rock run` builds before launching, so a separate `rock build` is optional. The server binds only to loopback. If binding fails, for example because port 8080 is in use, `main` reports failure and exits with status 1. Change the port in the source if needed.

Leave that terminal running and use a second terminal:

```console
$ curl --http1.1 --max-time 5 -i http://127.0.0.1:8080/health
HTTP/1.1 200 OK
Content-Type: text/plain
Content-Length: 3
Connection: close

ok
$ curl --http1.1 --max-time 5 -i --data-binary 'hello' http://127.0.0.1:8080/echo
HTTP/1.1 200 OK
Content-Type: application/octet-stream
Content-Length: 5
Connection: close

hello
```

The echo body has no trailing newline, so the next shell prompt may appear immediately after `hello`. `--data-binary` sends a POST request with a known content length. The example does not implement chunked request bodies.

Try the remaining behaviors:

```console
$ curl --http1.1 --max-time 5 -I http://127.0.0.1:8080/health
$ curl --http1.1 --max-time 5 -i http://127.0.0.1:8080/echo
$ curl --http1.1 --max-time 5 -i http://127.0.0.1:8080/missing
```

The HEAD request returns headers with `Content-Length: 3` but no body. GET `/echo` returns 405 with `Allow: POST`, and `/missing` returns 404. Stop the server with Ctrl-C when finished; this library does not expose a graceful-shutdown API.

## Ownership and Serving

`Server::bind address` uses `ServerConfig::default!`. `Server::bind_with_config address, config` uses explicit limits, as above. Both return `Result Server, HttpError`. `local_addr!` can report the bound address, including an operating-system-selected port when binding port zero; its error type is `IoError`.

`serve` consumes the server and shares the handler through `Arc`. Each accepted connection is moved into a detached worker thread. Workers may invoke the handler concurrently, so this tutorial uses a capture-free named function. Shared mutable application state needs appropriate synchronization; shared ownership alone is not a lock.

The return type of `serve` is `Result (), HttpError`, but normal serving keeps accepting connections indefinitely. Listener-accept and thread-spawn failures can escape to `main`. Request parsing, handler response validation, and socket-write failures inside an existing worker affect only that connection; the worker currently discards its result. An invalid application response does not automatically become a 500 response, and `main` is not a per-request error logger.

## Inspect Headers Without a Socket

Requests retain headers in order, including duplicates. `request.header name` returns the first matching value as `Option &Str`; `request.headers_named name` returns owned copies of every matching value as `Vec String`. Lookup compares names case-insensitively.

The parser can be used independently of `Server`. This complete alternative `src/main.rk` uses the same project manifest and illustrates duplicate lookup without opening a port:

```rock
> stdlib::string::str_as_bytes
> rock_http::config::ServerConfig
> rock_http::parser::DecodeResult
> rock_http::parser::parse_request

main = ->
    config = ServerConfig::default!
    bytes = str_as_bytes "GET / HTTP/1.1\r\nHost: local\r\nX-Tag: one\r\nX-Tag: two\r\n\r\n"
    match parse_request bytes, &config
        DecodeResult::Complete request, consumed =>
            match request.header "HOST"
                Option::Some value => value.println!
                Option::None => "missing host".println!
            tags = request.headers_named "x-tag"
            tags.len!.println!
            0
        DecodeResult::Incomplete => 1
        DecodeResult::Error _ => 2
```

This prints `local` and `2`. `DecodeResult::Complete` also reports the consumed byte count, useful when a caller has buffered additional data. `Incomplete` means the parser needs more bytes, not that it remembers the previous call: supply the accumulated buffer again. `Server` does this buffering for you but handles only one request per connection and does not serve any trailing pipelined requests.

## Limits and Protocol Scope

The default configuration and the meaning of its fields are:

| Field | Default | What is limited |
| --- | --- | --- |
| `max_request_line_bytes` | 8192 | Request line before its CRLF terminator |
| `max_header_bytes` | 32768 | Entire request head, including request line and final CRLF CRLF |
| `max_headers` | 100 | Number of header fields, counting duplicates |
| `max_body_bytes` | 1048576 | Declared `Content-Length` body size |

Our server lowers the body limit to 65536 bytes. Choose nonnegative, sensible limits; the configuration constructor does not validate them. These are parser limits, not a total process-memory budget: requests and bodies are buffered, a worker reads in 4096-byte chunks, and multiple connections allocate independently.

The parser accepts HTTP/1.0 and HTTP/1.1 request lines with CRLF separators and fixed-length bodies. A missing `Content-Length` means an empty body, not a body delimited by closing the connection. Repeated content lengths are accepted only when their numeric values agree. Header names must be HTTP tokens; values currently permit printable ASCII and horizontal tab, not arbitrary non-ASCII bytes. The parser is not a complete HTTP conformance validator, including for Host requirements.

The server maps request failures to these responses, where it can still write to the connection:

| Status | Examples |
| --- | --- |
| 400 | Malformed request, conflicting or invalid content length, unsupported version, oversized request line |
| 413 | Declared body exceeds the configured limit |
| 431 | Request head exceeds the byte limit or header count limit |
| 501 | Any `Transfer-Encoding` header, including chunked coding |

Response serialization validates status values in `100..=999`, header names and values, and any supplied content length before writing. Let it calculate `Content-Length`; a mismatched manual value is rejected. Transfer-encoding response headers are rejected too. The writer suppresses bodies for HEAD and for 1xx, 204, and 304 statuses; HEAD normally keeps the corresponding body's length, while those body-forbidden statuses are framed with length zero in this implementation.

Responses use HTTP/1.1 status lines and default to `Connection: close`. Do not override that header with `keep-alive`: the server still closes after one response. There is no keep-alive, pipelining, TLS, compression, multipart parser, WebSocket upgrade, async runtime, HTTP/2, or HTTP/3 support. There is also no automatic `100 Continue` handshake; clients should send fixed-length bodies without waiting for one.

Finally, safe memory access does not make an unbounded service safe to expose publicly. There are no read deadlines, connection-count limits, worker-pool bounds, or built-in backpressure. A slow client can occupy a blocking worker, and a large number of clients can exhaust resources even when every individual request is within its limits. Keep this tutorial on loopback and treat deployment hardening as a separate responsibility.

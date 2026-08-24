[![Cargo](https://img.shields.io/crates/v/reqwest-streams.svg)](https://crates.io/crates/reqwest-streams)
![tests and formatting](https://github.com/abdolence/reqwest-streams-rs/workflows/tests%20&amp;%20formatting/badge.svg)
![security audit](https://github.com/abdolence/reqwest-streams-rs/workflows/security%20audit/badge.svg)

# reqwest streams for Rust

Library provides HTTP response streaming support for [reqwest](https://github.com/seanmonstar/reqwest):
- JSON array stream format
- JSON lines stream format
- CSV stream
- Protobuf len-prefixed stream format
- Arrow IPC stream format

This type of responses are useful when you are reading huge stream of objects from some source (such as database, file, etc)
and want to avoid huge memory allocation.

## Quick start

Cargo.toml:
```toml
[dependencies]
reqwest-streams = { version = "0.19", features=["json", "csv", "protobuf", "arrow"] }
```

Example code:
```rust

use reqwest_streams::*;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
struct MyTestStructure {
    some_test_field: String
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {

    let _stream = reqwest::get("http://localhost:8080/json-array")
        .await?
        .json_array_stream::<MyTestStructure>(1024);

    Ok(())
}
```

All examples available in [examples](examples) directory.

To run example use:
```
# cargo run --example json-stream
```

## Streaming uploads

The same formats work the other way round: give a `POST` or `PUT` a stream of items and it is
encoded into the request body as it is sent, without ever holding the whole thing in memory.

```rust
use reqwest_streams::JsonStreamRequest;

client
    .post("http://localhost:8080/ingest")
    .json_array_stream_body(items)
    .send()
    .await?;
```

There is one method per format — `json_array_stream_body`, `json_nl_stream_body`,
`csv_stream_body`, `protobuf_stream_body`, `arrow_ipc_stream_body` — each with a `try_` variant
taking a fallible source stream. The `Content-Type` is set from the format.

For options, or for a body you want to hand to `multipart` or a hand-built request, build a
`ReqwestStreamBody` and pass it to `.stream_body(...)`:

```rust
let body = ReqwestStreamBody::with_options(
    JsonNewLineStreamFormat::new(),
    items,
    ReqwestStreamBodyOptions::new()
        .buffering_bytes(8 * 1024)
        .on_progress(|p| metrics::counter!("uploaded_bytes").increment(p.bytes)),
);

client.post(url).stream_body(body).send().await?;
```

`buffering_bytes` is worth setting for formats with small items: without it, JSON Lines of
short objects emits one chunked-transfer frame per item.

### Caveats

Streaming a *request* body is much less universally supported than streaming a response. None
of this stops it working, but each will surprise you if unexpected:

1. **The body cannot be replayed.** `try_clone()` returns `None`, so retry middleware cannot
   retry the request.
2. **A redirect silently sends an empty body.** `reqwest` follows redirects through a layer
   that substitutes a default body when the original cannot be cloned — an *empty* one, with no
   error. **Use `redirect::Policy::none()` for streaming uploads.** This is the sharpest edge
   here and the least obvious.
3. **Transfer-Encoding is chunked**, since no `Content-Length` can be computed. Some API
   gateways reject chunked request bodies. HTTP/2 is unaffected.
4. **`Expect: 100-continue` is not supported** by hyper, so you may upload a great deal before
   learning the request was rejected. When the server answers early the body is dropped and the
   outcome is reported as `aborted`.
5. **Buffering reverse proxies defeat streaming.** nginx buffers request bodies by default;
   set `proxy_request_buffering off;`.
6. **Timeouts cover the whole exchange**, so a slow *source* stream can trip
   `RequestBuilder::timeout`.

Errors from your source stream abort the request. When that happens hyper usually flattens the
cause away, so `send()` returns a generic transport error — `ReqwestStreamBodyOptions::on_error`
is often the only way to see what actually failed.

## Observing errors

An error that happens mid-stream is yielded as an item, so a consumer that stops at the first
one — `try_collect()`, or `?` inside a loop — silently ends up with a truncated result and no
indication of why.

Use `on_error` to observe them. It is called for every error, both transport errors and
decoding errors produced by the format itself:

```rust
    response.json_array_stream_with_options::<MyTestStructure>(
        ReqwestStreamOptions::new()
            .max_obj_len(1024)
            .on_error(|err| tracing::error!("Stream failed: {err}")),
    )
```

Alternatively, enable the `tracing` feature to have the library log them for you:

```toml
reqwest-streams = { version = "0.19", features = ["json", "tracing"] }
```

Errors are then logged at the `ERROR` level on the `http_streams_core` target, so they can be
filtered with `RUST_LOG=http_streams_core=off`. Both the log event and your `on_error`
callback fire when the feature is enabled and a callback is set.

Note that an error is not necessarily the end of the stream. The JSON Lines and CSV formats
produce their decoding errors from a successfully framed line, so reading resumes with the next
one; a malformed frame, by contrast, does end the stream.

## Observing progress

A response is read lazily, long after the call that created it returned, so nothing at the call
site can tell you how much of it actually arrived. Enable the `tracing` feature to have the
library report that for you:

```toml
reqwest-streams = { version = "0.19", features = ["json", "tracing"] }
```

At `INFO` every stream reports its totals once, when it ends:

```text
INFO http_streams_core::stream{format="json_array" direction="response" side="client" status=200 buf_capacity=8192 max_obj_len=65536 items=300 bytes=8891 errors=0 elapsed_ms=3396 outcome="completed"}: Finished streaming an HTTP body items=300 bytes=8891 errors=0 elapsed_ms=3396 outcome="completed"
```

The `outcome` tells apart the three ways a stream can end: `completed`, `aborted` (the consumer
stopped reading early, which is otherwise invisible), and `failed`, which reports at `ERROR`
instead, alongside the errors themselves. A stream that was built but never polled reports
nothing at all.

Raise it to `RUST_LOG=reqwest_streams=debug,http_streams_core=debug` and long-running streams
additionally report progress about once a second:

```text
DEBUG http_streams_core::stream{format="json_array" status=200}: Streaming an HTTP body items=45 bytes=1295 elapsed_ms=510
DEBUG http_streams_core::stream{format="json_array" status=200}: Streaming an HTTP body items=90 bytes=2600 elapsed_ms=1020
INFO  http_streams_core::stream{format="json_array" status=200 items=300 bytes=8891 errors=0 elapsed_ms=3396 outcome="completed"}: Finished streaming an HTTP body ...
```

Everything is recorded on an `http_streams_core::stream` span, created while your own
span is still current, so collectors nest it under your request and read `items`, `bytes`,
`errors`, `elapsed_ms` and `outcome` as span attributes rather than as log text. The span also
carries the response `status` and, when the server sent one, its `content_length`, which is
what lets you turn `bytes` into a completion percentage. Use `http_streams_core=trace` to
additionally get an event per body chunk.

The target is `http_streams_core` rather than `reqwest_streams` because the accounting is
shared with the server-side crate; the `direction` and `side` span fields tell the four cases
apart. Name both targets in your filter, so that anything this crate logs itself stays visible:

```text
RUST_LOG=reqwest_streams=debug,http_streams_core=debug
```

Reporting is time-based by default, so the number of lines is bound by how long a stream runs
and not by how much it carries. Both triggers are configurable, and progress is also reported
whenever the item count crosses a step if you ask for one:

```rust
    response.json_array_stream_with_options::<MyTestStructure>(
        ReqwestStreamOptions::new()
            .progress_interval(std::time::Duration::from_secs(5))
            .progress_items(100_000),
    )
```

The same accounting is available without tracing, for metrics:

```rust
    response.json_array_stream_with_options::<MyTestStructure>(
        ReqwestStreamOptions::new().on_progress(|progress| {
            if progress.outcome != ReqwestStreamOutcome::InProgress {
                metrics::counter!("streamed_bytes").increment(progress.bytes);
            }
        }),
    )
```

`items` counts the objects successfully decoded, so an item that failed to decode is not
counted, and an item is whatever the format produces: for the Arrow format that is a
`RecordBatch`, not a row. `bytes` counts what reqwest handed over, which is *after* transfer-
and content-decoding — with reqwest's `gzip` or `brotli` features enabled that is the
decompressed size, not the size on the wire.

Note that `ReqwestStreamOptions::new()` does **not** limit object size: unlike the
positional-argument methods, which make you choose, `max_obj_len` defaults to `usize::MAX`. Set
it explicitly when reading from a source you do not control.

Nothing is counted at all unless something is listening: with no `on_error`, no `on_progress`
and no subscriber interested in `reqwest_streams`, the stream pipeline is left untouched.

## Need server support?

[axum-streams](https://github.com/abdolence/axum-streams-rs) is the other half of the pair, and
covers both directions too. Since its 0.29 it can also *receive* a streamed request body, so an
upload sent with `json_nl_stream_body` and friends is decoded on the server by its
`StreamBodyFrom` extractor:

```rust
// Client, this crate
client.post(url).json_nl_stream_body(items).send().await?;

// Server, axum-streams
async fn ingest(mut items: JsonNlStreamFrom<MyItem>) -> Json<u64> { /* ... */ }
```

Both crates encode and decode through the same
[http-streams-core](https://github.com/abdolence/http-streams-core-rs), so the two sides cannot
drift apart.

## Upgrading to 0.19

The wire formats now live in [`http-streams-core`](https://github.com/abdolence/http-streams-core-rs)
and are shared with [axum-streams](https://github.com/abdolence/axum-streams-rs), so both sides
of a stream are encoded and decoded by one implementation. What is visible:

- **`StreamBodyError` and `StreamBodyKind` are re-exported unchanged**, at the same paths and
  with the same variant names. `StreamBodyKind` is now `#[non_exhaustive]` and has gained a
  `MaxBodyLenReachedError` variant used by the server side, so a `match` over it needs a `_` arm.
- **Tracing moved to the `http_streams_core` target** and to an `http_streams_core::stream`
  span. `RUST_LOG=reqwest_streams=debug` no longer selects it on its own; use
  `RUST_LOG=reqwest_streams=debug,http_streams_core=debug`.
  Client and server are told apart by the `side` span field.
Three latent bugs are fixed along the way:

- **CSV decoding no longer corrupts quoted fields.** Framing now goes through `csv-core`
  rather than line splitting, which fixes two kinds of silent data loss: a quoted field
  containing a **newline** was truncated (and the surviving row reported *no error*), and a
  **backslash** inside a quoted field was eaten as an escape, even though the encoder escapes
  by doubling quotes rather than with backslashes. CSV decoding also no longer allocates an
  8 KiB buffer per row.
- **The protobuf decoder no longer drops trailing messages.** When two or more complete frames
  were buffered as the body ended, everything after the first was lost without an error.
- **Zero-length protobuf messages frame correctly.** A message whose fields all hold their
  defaults encodes to zero bytes, and its length prefix was being confused with the next
  frame's.

## Licence
Apache Software License (ASL)

## Author
Abdulla Abdurakhmanov

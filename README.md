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
reqwest-streams = { version = "0.17", features=["json", "csv", "protobuf", "arrow"] }
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
reqwest-streams = { version = "0.17", features = ["json", "tracing"] }
```

Errors are then logged at the `ERROR` level on the `reqwest_streams` target, so they can be
filtered with the usual `RUST_LOG=reqwest_streams=off`. Both the log event and your `on_error`
callback fire when the feature is enabled and a callback is set.

Note that an error is not necessarily the end of the stream. The JSON Lines and CSV formats
produce their decoding errors from a successfully framed line, so reading resumes with the next
one; a malformed frame, by contrast, does end the stream.

## Observing progress

A response is read lazily, long after the call that created it returned, so nothing at the call
site can tell you how much of it actually arrived. Enable the `tracing` feature to have the
library report that for you:

```toml
reqwest-streams = { version = "0.17", features = ["json", "tracing"] }
```

At `INFO` every stream reports its totals once, when it ends:

```text
INFO reqwest_streams::response_stream{format="json_array" status=200 buf_capacity=8192 max_obj_len=65536 items=300 bytes=8891 errors=0 elapsed_ms=3396 outcome="completed"}: Finished streaming an HTTP body items=300 bytes=8891 errors=0 elapsed_ms=3396 outcome="completed"
```

The `outcome` tells apart the three ways a stream can end: `completed`, `aborted` (the consumer
stopped reading early, which is otherwise invisible), and `failed`, which reports at `ERROR`
instead, alongside the errors themselves. A stream that was built but never polled reports
nothing at all.

Raise it to `RUST_LOG=reqwest_streams=debug` and long-running streams additionally report
progress about once a second:

```text
DEBUG reqwest_streams::response_stream{format="json_array" status=200}: Streaming an HTTP body items=45 bytes=1295 elapsed_ms=510
DEBUG reqwest_streams::response_stream{format="json_array" status=200}: Streaming an HTTP body items=90 bytes=2600 elapsed_ms=1020
INFO  reqwest_streams::response_stream{format="json_array" status=200 items=300 bytes=8891 errors=0 elapsed_ms=3396 outcome="completed"}: Finished streaming an HTTP body ...
```

Everything is recorded on a `reqwest_streams::response_stream` span, created while your own
span is still current, so collectors nest it under your request and read `items`, `bytes`,
`errors`, `elapsed_ms` and `outcome` as span attributes rather than as log text. The span also
carries the response `status` and, when the server sent one, its `content_length` — which is
what lets you turn `bytes` into a completion percentage. Use `reqwest_streams=trace` to
additionally get an event per body chunk.

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
There is the same functionality:
- [axum-streams](https://github.com/abdolence/axum-streams-rs).

## Licence
Apache Software License (ASL)

## Author
Abdulla Abdurakhmanov

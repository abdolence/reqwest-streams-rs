#![allow(unused_parens, clippy::new_without_default)]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! HTTP body streaming support for reqwest, in both directions, for different formats:
//! - JSON array stream format
//! - JSON Lines (NL/NewLines) format
//! - CSV stream format
//! - [Protobuf] len-prefixed stream format
//! - [Apache Arrow IPC] stream format
//!
//! This type of responses are useful when you are reading huge stream of objects from some source (such as database, file, etc)
//! and want to avoid huge memory allocations to store on the server side.
//!
//! # Features
//!
//! **Note:** The `default` features do not include any formats.
//!
//! - `json`: JSON array and JSON Lines (JSONL) stream formats
//! - `csv`: CSV stream format
//! - `protobuf`: [Protobuf] len-prefixed stream format
//! - `arrow`: [Apache Arrow IPC] stream format
//! - `tracing`: report progress and errors through [tracing]
//!
//! # Example
//!
//! ```rust,no_run
//! use futures::stream::BoxStream as _;
//! use reqwest_streams::JsonStreamResponse as _;
//! use serde::Deserialize;
//!
//! #[derive(Debug, Clone, Deserialize)]
//! struct MyTestStructure {
//!     some_test_field: String
//! }
//!
//!#[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!
//!     let _stream = reqwest::get("http://localhost:8080/json-array")
//!         .await?
//!         .json_array_stream::<MyTestStructure>(1024);
//!
//!     Ok(())
//! }
//! ```
//!
//! # Streaming uploads
//!
//! The same formats work the other way round: give a `POST` or `PUT` a stream of items and it
//! is encoded into the request body as it is sent, without ever holding the whole thing in
//! memory.
//!
//! ```rust,no_run
//! use futures::stream;
//! use reqwest_streams::JsonStreamRequest as _;
//! use serde::Serialize;
//!
//! #[derive(Serialize)]
//! struct MyTestStructure {
//!     some_test_field: String
//! }
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let items = stream::iter(vec![MyTestStructure { some_test_field: "value".into() }]);
//!
//! reqwest::Client::new()
//!     .post("http://localhost:8080/ingest")
//!     .json_array_stream_body(items)
//!     .send()
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! The `Content-Type` is set from the format. Use [`ReqwestStreamBody`] directly when you need
//! options, a body for `multipart`, or a request built by hand.
//!
//! **Read [`ReqwestStreamBody`]'s caveats before using this in anger.** Streaming a request
//! body is much less universally supported than streaming a response: the body cannot be
//! retried, a redirect silently sends an *empty* body, and buffering reverse proxies defeat
//! the streaming entirely.
//!
//! More and complete examples available on the github in the examples directory.
//!
//! ## Need server support?
//! [axum-streams](https://github.com/abdolence/axum-streams-rs) is the other half of the pair,
//! and covers both directions too. Since its 0.29 it can also *receive* a streamed request
//! body, so an upload sent with [`JsonStreamRequest`] and friends is decoded on the server by
//! its `StreamBodyFrom` extractor. Both crates encode and decode through the same
//! [http-streams-core](https://github.com/abdolence/http-streams-core-rs).
//!
//!
//! # Observing stream errors
//!
//! An error that happens mid-stream is yielded as an item, so a consumer that stops at the
//! first one silently gets a truncated result. Use [`ReqwestStreamOptions::on_error`] to
//! observe them, or enable the `tracing` feature to have them logged at `ERROR` on the
//! `reqwest_streams` target.
//!
//! # Observing stream progress
//!
//! Nothing at the call site can tell you how much of a response actually arrived, because it
//! is read long after the call returned. With the `tracing` feature every stream reports its
//! totals once at `INFO` when it ends, on an `http_streams_core::stream` span:
//!
//! ```text
//! INFO http_streams_core::stream{format="json_array" direction="response" side="client" status=200 items=1000 bytes=28001 errors=0 elapsed_ms=11239 outcome="completed"}: Finished streaming an HTTP body
//! ```
//!
//! The `outcome` tells apart the three ways a stream can end: `completed`, `aborted` (the
//! consumer stopped reading early) and `failed`, which reports at `ERROR` instead. Raise the
//! filter to `reqwest_streams=debug,http_streams_core=debug` for a progress line about once a
//! second, and to `http_streams_core=trace` for one per body chunk. Naming both targets keeps
//! anything this crate logs itself visible alongside the shared accounting.
//!
//! The target is `http_streams_core` rather than `reqwest_streams` because the accounting is
//! shared with `axum-streams`; the `direction` and `side` span fields tell the cases apart.
//!
//! The same accounting is available without tracing, for metrics, via
//! [`ReqwestStreamOptions::on_progress`].
//!
//! [tracing]: https://docs.rs/tracing
//! [Apache Arrow IPC]: https://arrow.apache.org/docs/format/Columnar.html#serialization-and-interprocess-communication-ipc
//! [Protobuf]: https://protobuf.dev/programming-guides/encoding/

#[macro_use]
mod macros;

cfg_json! {
    pub use json_body::JsonStreamRequest;
    pub use json_stream::JsonStreamResponse;
    mod json_body;
    mod json_stream;
}

cfg_csv! {
    pub use csv_body::CsvStreamRequest;
    pub use csv_stream::CsvStreamResponse;
    mod csv_body;
    mod csv_stream;
}

use crate::error::StreamBodyError;

cfg_formats! {
    pub use stream_body::{ReqwestStreamBody, ReqwestStreamBodyOptions, StreamBodyRequest};
    mod stream_body;
}

cfg_formats! {
    pub use observability::{
        ReqwestStreamErrorHandler, ReqwestStreamOptions, ReqwestStreamOutcome,
        ReqwestStreamProgress, ReqwestStreamProgressHandler,
    };
    mod observability;
}

cfg_protobuf! {
    pub use protobuf_body::ProtobufStreamRequest;
    pub use protobuf_stream::ProtobufStreamResponse;
    mod protobuf_body;
    mod protobuf_stream;
}

cfg_arrow! {
    pub use arrow_body::ArrowIpcStreamRequest;
    pub use arrow_ipc_stream::ArrowIpcStreamResponse;
    mod arrow_body;
    mod arrow_ipc_stream;
}

pub mod error;

/// Alias for the [`Result`] type returned by streaming responses.
pub type StreamBodyResult<T> = std::result::Result<T, StreamBodyError>;

/// The shared core, re-exported so downstream code can name the types this crate's public API
/// mentions without adding its own dependency — and so it cannot end up with a second,
/// incompatible copy of them.
pub use http_streams_core;

cfg_formats! {
    // Only the format modules' tests use it, so with no format enabled there is no caller.
    #[cfg(test)]
    mod test_client;
}

#![allow(unused_parens, clippy::new_without_default)]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

//! Streaming responses support for reqwest for different formats:
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
//! More and complete examples available on the github in the examples directory.
//!
//! ## Need server support?
//! There is the same functionality:
//! - [axum-streams](https://github.com/abdolence/axum-streams-rs).
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
//! totals once at `INFO` when it ends, on a `reqwest_streams::response_stream` span:
//!
//! ```text
//! INFO reqwest_streams::response_stream{format="json_array" status=200 items=1000 bytes=28001 elapsed_ms=11239 outcome="completed"}: Finished streaming an HTTP body
//! ```
//!
//! The `outcome` tells apart the three ways a stream can end: `completed`, `aborted` (the
//! consumer stopped reading early) and `failed`, which reports at `ERROR` instead. Raise the
//! filter to `reqwest_streams=debug` for a progress line about once a second, and to
//! `reqwest_streams=trace` for one per body chunk.
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
    pub use json_stream::JsonStreamResponse;
    mod json_stream;
    mod json_array_codec;
}

cfg_csv! {
    pub use csv_stream::CsvStreamResponse;
    mod csv_stream;
}

use crate::error::StreamBodyError;

cfg_formats! {
    pub use observability::{
        ReqwestStreamErrorHandler, ReqwestStreamOptions, ReqwestStreamOutcome,
        ReqwestStreamProgress, ReqwestStreamProgressHandler,
    };
    mod observability;
}

cfg_protobuf! {
    pub use protobuf_stream::ProtobufStreamResponse;
    mod protobuf_stream;
    mod protobuf_len_codec;
}

cfg_arrow! {
    pub use arrow_ipc_stream::ArrowIpcStreamResponse;
    mod arrow_ipc_stream;
    mod arrow_ipc_len_codec;
}

pub mod error;

/// Alias for the [`Result`] type returned by streaming responses.
pub type StreamBodyResult<T> = std::result::Result<T, StreamBodyError>;

cfg_formats! {
    // Only the format modules' tests use it, so with no format enabled there is no caller.
    #[cfg(test)]
    mod test_client;
}

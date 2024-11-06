#![allow(unused_parens, clippy::new_without_default)]
#![forbid(unsafe_code)]

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
//! [Apache Arrow IPC]: https://arrow.apache.org/docs/format/Columnar.html#serialization-and-interprocess-communication-ipc
//! [Protobuf]: https://protobuf.dev/programming-guides/encoding/

#[cfg(feature = "json")]
mod json_stream;
#[cfg(feature = "json")]
pub use json_stream::JsonStreamResponse;
#[cfg(feature = "json")]
mod json_array_codec;

#[cfg(feature = "csv")]
mod csv_stream;
#[cfg(feature = "csv")]
pub use csv_stream::CsvStreamResponse;

use crate::error::StreamBodyError;

#[cfg(feature = "protobuf")]
mod protobuf_stream;
#[cfg(feature = "protobuf")]
pub use protobuf_stream::ProtobufStreamResponse;
#[cfg(feature = "protobuf")]
mod protobuf_len_codec;

#[cfg(feature = "arrow")]
mod arrow_ipc_stream;
#[cfg(feature = "arrow")]
pub use arrow_ipc_stream::ArrowIpcStreamResponse;
#[cfg(feature = "arrow")]
mod arrow_ipc_len_codec;

pub mod error;

/// Alias for the [`Result`] type returned by streaming responses.
pub type StreamBodyResult<T> = std::result::Result<T, StreamBodyError>;

#[cfg(test)]
mod test_client;

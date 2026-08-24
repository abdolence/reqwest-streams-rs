//! Error types for streaming responses.
//!
//! These now live in [`http_streams_core`] and are shared with `axum-streams`, so that a body
//! encoded by one and decoded by the other reports failures through one type. They are
//! re-exported here under the names they have always had.
//!
//! Two changes are visible: [`StreamBodyKind`] is now `#[non_exhaustive]`, and it gained a
//! `MaxBodyLenReachedError` variant used by the server side. A `match` over it needs a `_` arm.

pub use http_streams_core::error::{
    StreamError as StreamBodyError, StreamErrorKind as StreamBodyKind,
};

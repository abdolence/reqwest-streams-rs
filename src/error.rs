//! Error types for streaming responses.

use std::fmt;

type BoxedError = Box<dyn std::error::Error + Send + Sync>;

/// The error that may occur when attempting to stream a [`reqwest::Response`].
pub struct StreamBodyError {
    kind: StreamBodyKind,
    source: Option<BoxedError>,
    message: Option<String>,
}

impl StreamBodyError {
    /// Create a new instance of an error.
    pub fn new(kind: StreamBodyKind, source: Option<BoxedError>, message: Option<String>) -> Self {
        Self {
            kind,
            source,
            message,
        }
    }

    /// The kind of error that occurred during streaming.
    pub fn kind(&self) -> StreamBodyKind {
        self.kind
    }

    /// The actual error that occurred.
    pub fn source(&self) -> Option<&BoxedError> {
        self.source.as_ref()
    }

    /// The message associated with the error.
    pub fn message(&self) -> Option<&str> {
        self.message.as_deref()
    }
}

/// The kind of error that occurred during streaming.
#[derive(Clone, Copy, Debug)]
pub enum StreamBodyKind {
    /// An error occured while decoding a frame or format.
    CodecError,

    /// An error occured while reading the stream.
    InputOutputError,

    /// The maximum object length was exceeded.
    MaxLenReachedError,
}

impl fmt::Debug for StreamBodyError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut builder = f.debug_struct("reqwest::Error");

        builder.field("kind", &self.kind);

        if let Some(ref source) = self.source {
            builder.field("source", source);
        }

        if let Some(ref message) = self.message {
            builder.field("message", message);
        }

        builder.finish()
    }
}

impl fmt::Display for StreamBodyError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.kind {
            StreamBodyKind::CodecError => f.write_str("Frame/codec error")?,
            StreamBodyKind::InputOutputError => f.write_str("I/O error")?,
            StreamBodyKind::MaxLenReachedError => f.write_str("Max object length reached")?,
        };

        if let Some(message) = &self.message {
            write!(f, ": {}", message)?;
        }

        if let Some(e) = &self.source {
            write!(f, ": {}", e)?;
        }

        Ok(())
    }
}

impl std::error::Error for StreamBodyError {}

impl From<std::io::Error> for StreamBodyError {
    fn from(err: std::io::Error) -> Self {
        StreamBodyError::new(StreamBodyKind::InputOutputError, Some(Box::new(err)), None)
    }
}

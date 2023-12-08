[![Cargo](https://img.shields.io/crates/v/reqwest-streams.svg)](https://crates.io/crates/reqwest-streams)
![tests and formatting](https://github.com/abdolence/reqwest-streams-rs/workflows/tests%20&amp;%20formatting/badge.svg)
![security audit](https://github.com/abdolence/reqwest-streams-rs/workflows/security%20audit/badge.svg)

# reqwest streams for Rust

Library provides HTTP response streaming support for [reqwest](https://github.com/seanmonstar/reqwest):
- JSON array stream format
- JSON lines stream format
- CSV stream
- Protobuf len-prefixed stream format

This type of responses are useful when you are reading huge stream of objects from some source (such as database, file, etc)
and want to avoid huge memory allocation.

## Quick start

Cargo.toml:
```toml
[dependencies]
reqwest-streams = { version = "0.5", features=["json", "csv", "protobuf"] }
```

Example code:
```rust

use reqwest_streams::*;
use futures_util::stream::BoxStream;
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

## Need server support?
There is the same functionality:
- [axum-streams](https://github.com/abdolence/axum-streams-rs).

## Licence
Apache Software License (ASL)

## Author
Abdulla Abdurakhmanov

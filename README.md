# slack-morphism-reqwest

[![crates.io](https://img.shields.io/crates/v/slack-morphism-reqwest.svg)](https://crates.io/crates/slack-morphism-reqwest)
[![docs.rs](https://docs.rs/slack-morphism-reqwest/badge.svg)](https://docs.rs/slack-morphism-reqwest)
[![License](https://img.shields.io/crates/l/slack-morphism-reqwest.svg)](#license)

A [reqwest][reqwest]-based HTTP connector for the [slack-morphism][sm] Rust SDK.

`slack-morphism` abstracts its HTTP layer through the `SlackClientHttpConnector` trait. The official crate ships a [hyper][hyper]-backed connector under the `hyper` feature flag. This crate is an alternative connector for projects that already use `reqwest` elsewhere and want to keep a single HTTP stack in the binary.

## When to use this

- Your project already pulls in `reqwest` (so adding hyper just for slack-morphism would mean two HTTP stacks)
- You want full control over your HTTP client (timeouts, proxies, user-agents, redirects, headers, TLS) and prefer reqwest's builder API
- You're shipping in an environment where binary size matters and the hyper transport is overhead

If neither of those applies, the official `slack-morphism` hyper connector is a fine default.

## Installation

```toml
[dependencies]
slack-morphism = "2.20"
slack-morphism-reqwest = "0.1"
reqwest = { version = "0.12", features = ["rustls-tls", "json", "gzip"] }
```

The `gzip` feature on your `reqwest` is only needed if you build your own `reqwest::Client` with `.gzip(true)` (see the example below). `ReqwestConnector::with_defaults` enables gzip on its internal client without needing it in your direct dep.

`slack-morphism-reqwest` depends on `slack-morphism` with `default-features = false` so you get the typed API + Socket Mode types without dragging in hyper.

## Usage

### With sensible defaults

```rust,no_run
use slack_morphism::prelude::*;
use slack_morphism_reqwest::ReqwestConnector;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let connector = ReqwestConnector::with_defaults()?;
    let client = SlackClient::new(connector);

    let token = SlackApiToken::new("xoxb-...".into());
    let session = client.open_session(&token);

    let response = session.auth_test().await?;
    println!("authed as {}", response.user.value());

    Ok(())
}
```

### With your own `reqwest::Client`

```rust,no_run
use std::time::Duration;
use slack_morphism::prelude::*;
use slack_morphism_reqwest::ReqwestConnector;

# fn build_client() -> Result<reqwest::Client, reqwest::Error> {
let http = reqwest::Client::builder()
    .user_agent("my-app/1.0")
    .timeout(Duration::from_secs(60))
    .gzip(true)
    .build()?;
# Ok(http) }

# fn use_client(http: reqwest::Client) {
let connector = ReqwestConnector::new(http);
let client = SlackClient::new(connector);
# }
```

### Pointing at a custom Slack API URL (for tests)

```rust,no_run
use slack_morphism_reqwest::ReqwestConnector;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let connector = ReqwestConnector::with_defaults()?
    .with_slack_api_url("http://localhost:8080/api");
# Ok(()) }
```

## Behavior

This connector mirrors the response handling of `slack-morphism`'s official hyper connector:

- `200 OK` JSON → parse Slack envelope, return typed response or `ApiError` if `ok: false`
- `200 OK` non-JSON / `204 No Content` → parsed as empty `{}`
- `429 Too Many Requests` → `RateLimitError` with `retry_after` from the `Retry-After` header
- everything else → `HttpError` with the response body attached

### Rate limiting

This connector reports `429` as a `RateLimitError` with the `Retry-After` header parsed and surfaced. It does **not**:

- Retry rate-limited requests automatically
- Throttle outbound requests proactively based on Slack's per-method tier limits
- Coordinate rate limits across multiple `SlackClient` instances

If you need tier-aware proactive throttling (the official hyper connector's `with_rate_control(...)` feature), you have two options: build it on top of this connector using the `retry_after` from `RateLimitError`, or use `slack-morphism`'s official hyper connector which ships with `SlackTokioRateController` built in. We may add an optional throttler in a future release — track or open an issue if this matters to you.

## Compatibility

| `slack-morphism-reqwest` | `slack-morphism` | `reqwest`   | MSRV   |
|--------------------------|------------------|-------------|--------|
| `0.1.x`                  | `2.20.x`         | `0.12.x`    | 1.75   |

## Status

This is a **third-party** connector. It is not maintained by the [slack-morphism][sm] authors. Issues and contributions are welcome at the project's [GitHub repo](https://github.com/evanzi/slack-morphism-reqwest).

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <https://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual-licensed as above, without any additional terms or conditions.

[sm]: https://github.com/abdolence/slack-morphism-rust
[reqwest]: https://crates.io/crates/reqwest
[hyper]: https://crates.io/crates/hyper

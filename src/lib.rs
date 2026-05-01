//! A [reqwest]-based HTTP connector for [slack-morphism].
//!
//! `slack-morphism` abstracts its HTTP layer through the
//! [`SlackClientHttpConnector`] trait. The official crate ships a
//! [hyper]-backed connector under the `hyper` feature flag. This
//! crate provides an alternative connector backed by [reqwest], for
//! projects that already use reqwest elsewhere and want to keep a
//! single HTTP stack in the binary.
//!
//! Bring your own [`reqwest::Client`] — this crate does not pick
//! timeouts, proxies, user-agents, or TLS settings for you. A
//! convenience [`ReqwestConnector::with_defaults`] is provided for
//! cases where you don't care.
//!
//! # Example
//!
//! ```no_run
//! use slack_morphism::prelude::*;
//! use slack_morphism_reqwest::ReqwestConnector;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let connector = ReqwestConnector::with_defaults()?;
//! let client = SlackClient::new(connector);
//!
//! let token = SlackApiToken::new("xoxb-...".into());
//! let session = client.open_session(&token);
//!
//! let response = session.auth_test().await?;
//! println!("authed: {:?}", response);
//! # Ok(())
//! # }
//! ```
//!
//! # Status
//!
//! This is a third-party connector; it is not maintained by the
//! [slack-morphism] authors. Issues and PRs welcome at
//! <https://github.com/evanzi/slack-morphism-reqwest>.
//!
//! [hyper]: https://crates.io/crates/hyper
//! [reqwest]: https://crates.io/crates/reqwest
//! [slack-morphism]: https://crates.io/crates/slack-morphism
//! [`SlackClientHttpConnector`]: slack_morphism::SlackClientHttpConnector

#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]
#![forbid(unsafe_code)]

use bytes::Bytes;
use futures::future::{BoxFuture, FutureExt};
use rvstruct::ValueStruct;
use slack_morphism::errors::{
    map_serde_error, SlackClientApiError, SlackClientError, SlackClientHttpError,
    SlackClientHttpProtocolError, SlackClientProtocolError, SlackRateLimitError,
};
use slack_morphism::multipart_form::FileMultipartData;
use slack_morphism::{
    ClientResult, SlackApiToken, SlackClientApiCallContext, SlackClientHttpApiUri,
    SlackClientHttpConnector, SlackClientId, SlackClientSecret, SlackEnvelopeMessage,
};
use std::time::Duration;
use tracing::debug;
use url::Url;

/// A [`SlackClientHttpConnector`] backed by a [`reqwest::Client`].
///
/// Construct with [`ReqwestConnector::new`] (recommended — bring
/// your own `reqwest::Client`) or [`ReqwestConnector::with_defaults`]
/// (a sensible default client for callers who don't want to pick
/// settings).
///
/// # Token authorization
///
/// Per-call `Authorization: Bearer <token>` is added based on the
/// token in the [`SlackClientApiCallContext`] passed by
/// `slack-morphism`. Tokens are never logged.
///
/// # Rate limiting
///
/// This connector maps Slack's `429 Too Many Requests` responses
/// into [`SlackClientError::RateLimitError`] with `retry_after`
/// extracted from the `Retry-After` header. It does **not**:
///
/// - retry rate-limited requests automatically;
/// - throttle outbound requests proactively based on Slack's
///   per-method tier limits;
/// - coordinate rate limits across multiple `SlackClient` instances.
///
/// If you need proactive tier-aware throttling (the official hyper
/// connector's `with_rate_control(...)` feature), build it on top
/// of this connector using `retry_after` from `RateLimitError`, or
/// use `slack-morphism`'s own hyper connector.
#[derive(Clone, Debug)]
pub struct ReqwestConnector {
    http: reqwest::Client,
    slack_api_url: String,
}

impl ReqwestConnector {
    /// Wrap a caller-supplied [`reqwest::Client`].
    ///
    /// The client controls timeouts, proxies, user-agent, redirects,
    /// connection pooling, and TLS. Most production callers should
    /// use this constructor.
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            slack_api_url: SlackClientHttpApiUri::SLACK_API_URI_STR.to_string(),
        }
    }

    /// Build a connector with a sensible default `reqwest::Client`:
    ///
    /// - 30-second total request timeout (matches Slack's documented
    ///   Web API behavior; bump it if calling slow endpoints like
    ///   `search.messages` against large workspaces)
    /// - gzip response decoding enabled
    /// - HTTP/2 enabled (Slack's API supports it)
    ///
    /// TLS is whatever the active reqwest feature flags select. This
    /// crate enables `rustls-tls` on its own reqwest dependency, but
    /// if you supply your own `reqwest::Client` via
    /// [`ReqwestConnector::new`] it uses your client's TLS stack.
    ///
    /// Use [`ReqwestConnector::new`] when you need finer control.
    pub fn with_defaults() -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .gzip(true)
            .build()?;
        Ok(Self::new(http))
    }

    /// Override the base Slack API URL.
    ///
    /// Defaults to `https://slack.com/api`. Useful for tests against
    /// a mock server, or for routing through an internal proxy.
    pub fn with_slack_api_url(mut self, slack_api_url: impl Into<String>) -> Self {
        self.slack_api_url = slack_api_url.into();
        self
    }

    /// Borrow the underlying [`reqwest::Client`].
    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }
}

impl From<reqwest::Client> for ReqwestConnector {
    fn from(http: reqwest::Client) -> Self {
        Self::new(http)
    }
}

impl ReqwestConnector {
    /// Send an HTTP request and adapt the response into the shape
    /// `slack-morphism` expects.
    ///
    /// Status code routing mirrors `slack-morphism`'s own hyper
    /// connector:
    ///
    /// - `200 OK` JSON: parse the Slack envelope; if `error` is set
    ///   return [`SlackClientError::ApiError`], else deserialize the
    ///   typed response
    /// - `200 OK` non-JSON or `204 No Content`: deserialize from
    ///   `"{}"` (lets unit responses succeed)
    /// - `429 Too Many Requests`: [`SlackClientError::RateLimitError`]
    ///   with `Retry-After` extracted from the header
    /// - anything else: [`SlackClientError::HttpError`] with the
    ///   response body attached
    async fn send_request<RS>(
        &self,
        request: reqwest::Request,
        context: SlackClientApiCallContext<'_>,
    ) -> ClientResult<RS>
    where
        RS: for<'de> serde::de::Deserialize<'de>,
    {
        let uri_str = if context.is_sensitive_url {
            // Sensitive URLs (e.g. file uploads with embedded tokens
            // in query params) get redacted before logging.
            redacted_url(request.url())
        } else {
            request.url().to_string()
        };

        context.tracing_span.in_scope(|| {
            debug!(slack_uri = %uri_str, "Sending HTTP request to {}", uri_str);
        });

        let response = self
            .http
            .execute(request)
            .await
            .map_err(map_reqwest_error)?;

        let status = response.status();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_secs);
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let body = response.text().await.map_err(map_reqwest_error)?;

        context.tracing_span.in_scope(|| {
            debug!(
                slack_uri = %uri_str,
                slack_http_status = status.as_u16(),
                "Received HTTP response {}",
                status,
            );
        });

        // Match upstream hyper connector: a missing or unparseable
        // Content-Type is treated as JSON (upstream uses
        // `Option::iter().all(...)` which is `true` for `None`). This
        // matters for Slack endpoints that occasionally omit the
        // header on rate-limit responses.
        let is_json = content_type
            .as_deref()
            .and_then(|ct| ct.parse::<mime::Mime>().ok())
            .map(|mime| mime.type_() == mime::APPLICATION && mime.subtype() == mime::JSON)
            .unwrap_or(true);

        // slack-morphism lives on http::StatusCode; reqwest also
        // re-exports it. We check the integer to stay independent.
        match status.as_u16() {
            200 if is_json => {
                let envelope: SlackEnvelopeMessage =
                    serde_json::from_str(&body).map_err(|err| map_serde_error(err, Some(&body)))?;
                match envelope.error {
                    None => {
                        let decoded = serde_json::from_str(&body)
                            .map_err(|err| map_serde_error(err, Some(&body)))?;
                        Ok(decoded)
                    }
                    Some(slack_error) => Err(SlackClientError::ApiError(
                        SlackClientApiError::new(slack_error)
                            .opt_errors(envelope.errors)
                            .opt_warnings(envelope.warnings)
                            .with_http_response_body(body),
                    )),
                }
            }
            200 | 204 => serde_json::from_str("{}").map_err(|err| map_serde_error(err, Some("{}"))),
            429 if is_json => {
                let envelope: SlackEnvelopeMessage =
                    serde_json::from_str(&body).map_err(|err| map_serde_error(err, Some(&body)))?;
                Err(SlackClientError::RateLimitError(
                    SlackRateLimitError::new()
                        .opt_retry_after(retry_after)
                        .opt_code(envelope.error)
                        .opt_warnings(envelope.warnings)
                        .with_http_response_body(body),
                ))
            }
            429 => Err(SlackClientError::RateLimitError(
                SlackRateLimitError::new()
                    .opt_retry_after(retry_after)
                    .with_http_response_body(body),
            )),
            _ => {
                let http_status = http::StatusCode::from_u16(status.as_u16())
                    .unwrap_or(http::StatusCode::INTERNAL_SERVER_ERROR);
                Err(SlackClientError::HttpError(
                    SlackClientHttpError::new(http_status).with_http_response_body(body),
                ))
            }
        }
    }

    fn build_request(
        &self,
        method: reqwest::Method,
        full_uri: Url,
        token: Option<&SlackApiToken>,
    ) -> reqwest::RequestBuilder {
        let mut builder = self.http.request(method, full_uri);
        if let Some(token) = token {
            builder = builder.bearer_auth(token.token_value.value());
        }
        builder
    }
}

impl SlackClientHttpConnector for ReqwestConnector {
    fn create_method_uri_path(&self, method_relative_uri: &str) -> ClientResult<Url> {
        Ok(format!("{}/{}", self.slack_api_url, method_relative_uri).parse()?)
    }

    fn http_get_uri<'a, RS>(
        &'a self,
        full_uri: Url,
        context: SlackClientApiCallContext<'a>,
    ) -> BoxFuture<'a, ClientResult<RS>>
    where
        RS: for<'de> serde::de::Deserialize<'de> + Send + 'a + Send,
    {
        async move {
            let request = self
                .build_request(reqwest::Method::GET, full_uri, context.token)
                .build()
                .map_err(map_reqwest_error)?;
            self.send_request(request, context).await
        }
        .boxed()
    }

    fn http_get_with_client_secret<'a, RS>(
        &'a self,
        full_uri: Url,
        client_id: &'a SlackClientId,
        client_secret: &'a SlackClientSecret,
    ) -> BoxFuture<'a, ClientResult<RS>>
    where
        RS: for<'de> serde::de::Deserialize<'de> + Send + 'a + 'a + Send,
    {
        async move {
            let oauth_span = tracing::span!(tracing::Level::DEBUG, "Slack OAuth Get");
            let context = SlackClientApiCallContext {
                rate_control_params: None,
                token: None,
                tracing_span: &oauth_span,
                is_sensitive_url: false,
            };

            let request = self
                .http
                .request(reqwest::Method::GET, full_uri)
                .basic_auth(client_id.value(), Some(client_secret.value()))
                .build()
                .map_err(map_reqwest_error)?;
            self.send_request(request, context).await
        }
        .boxed()
    }

    fn http_post_uri<'a, RQ, RS>(
        &'a self,
        full_uri: Url,
        request_body: &'a RQ,
        context: SlackClientApiCallContext<'a>,
    ) -> BoxFuture<'a, ClientResult<RS>>
    where
        RQ: serde::ser::Serialize + Send + Sync,
        RS: for<'de> serde::de::Deserialize<'de> + Send + 'a + Send + 'a,
    {
        async move {
            let body =
                serde_json::to_string(request_body).map_err(|err| map_serde_error(err, None))?;
            let request = self
                .build_request(reqwest::Method::POST, full_uri, context.token)
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/json; charset=utf-8",
                )
                .body(body)
                .build()
                .map_err(map_reqwest_error)?;
            self.send_request(request, context).await
        }
        .boxed()
    }

    fn http_post_uri_multipart_form<'a, 'p, RS, PT, TS>(
        &'a self,
        full_uri: Url,
        file: Option<FileMultipartData<'p>>,
        params: &'p PT,
        context: SlackClientApiCallContext<'a>,
    ) -> BoxFuture<'a, ClientResult<RS>>
    where
        RS: for<'de> serde::de::Deserialize<'de> + Send + 'a + Send + 'a,
        PT: std::iter::IntoIterator<Item = (&'p str, Option<TS>)> + Clone,
        TS: AsRef<str> + 'p + Send,
    {
        // Build the multipart form eagerly — reqwest's Form is owned,
        // so we avoid lifetime gymnastics in the async block.
        let mut form = reqwest::multipart::Form::new();
        for (key, maybe_value) in params.clone().into_iter() {
            if let Some(value) = maybe_value {
                form = form.text(key.to_string(), value.as_ref().to_string());
            }
        }
        if let Some(file) = file {
            // slack-morphism's `FileMultipartData::name` doubles as
            // the filename in the upstream hyper impl. The form field
            // key is hardcoded to "file" to match what Slack's
            // files.upload expects.
            let part =
                reqwest::multipart::Part::bytes(file.data.to_vec()).file_name(file.name.clone());
            let part = match part.mime_str(&file.content_type) {
                Ok(p) => p,
                Err(err) => {
                    return futures::future::ready(Err(SlackClientError::HttpProtocolError(
                        SlackClientHttpProtocolError::new().with_cause(Box::new(err)),
                    )))
                    .boxed();
                }
            };
            form = form.part("file", part);
        }

        async move {
            let request = self
                .build_request(reqwest::Method::POST, full_uri, context.token)
                .multipart(form)
                .build()
                .map_err(map_reqwest_error)?;
            self.send_request(request, context).await
        }
        .boxed()
    }

    fn http_post_uri_binary<'a, 'p, RS>(
        &'a self,
        full_uri: Url,
        content_type: String,
        data: &'a [u8],
        context: SlackClientApiCallContext<'a>,
    ) -> BoxFuture<'a, ClientResult<RS>>
    where
        RS: for<'de> serde::de::Deserialize<'de> + Send + 'a + Send + 'a,
    {
        // `'p` is unused but matches the trait declaration to keep
        // signature parity with the upstream hyper connector.
        let _ = std::marker::PhantomData::<&'p ()>;
        let body = Bytes::copy_from_slice(data);
        async move {
            let request = self
                .build_request(reqwest::Method::POST, full_uri, context.token)
                .header(reqwest::header::CONTENT_TYPE, content_type)
                .body(body)
                .build()
                .map_err(map_reqwest_error)?;
            self.send_request(request, context).await
        }
        .boxed()
    }
}

/// Map a `reqwest::Error` into the right `SlackClientError` variant.
///
/// Decode errors → `ProtocolError`. Everything else (timeouts,
/// connect errors, redirects, body errors) → `HttpProtocolError`
/// preserving the underlying cause.
fn map_reqwest_error(err: reqwest::Error) -> SlackClientError {
    if err.is_decode() {
        // serde_json error wrapped by reqwest — surface as a protocol
        // error so callers can distinguish from network issues.
        SlackClientError::ProtocolError(SlackClientProtocolError::new(serde_json::Error::io(
            std::io::Error::other(err.to_string()),
        )))
    } else {
        SlackClientError::HttpProtocolError(
            SlackClientHttpProtocolError::new().with_cause(Box::new(err)),
        )
    }
}

fn redacted_url(url: &Url) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    url.path().hash(&mut hasher);
    let hash = hasher.finish();
    format!(
        "{}://{}/-redacted-/{}",
        url.scheme(),
        url.host_str().unwrap_or("unknown-host"),
        hash
    )
}

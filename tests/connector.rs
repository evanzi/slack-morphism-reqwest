//! Integration tests for `ReqwestConnector` against a wiremock
//! HTTP server. Each test points the connector at the mock's URL via
//! `with_slack_api_url` so we exercise the real reqwest stack without
//! hitting Slack.

use rvstruct::ValueStruct;
use serde::{Deserialize, Serialize};
use slack_morphism::errors::SlackClientError;
use slack_morphism::multipart_form::FileMultipartData;
use slack_morphism::{
    ClientResult, SlackApiToken, SlackClient, SlackClientApiCallContext, SlackClientHttpConnector,
    SlackClientId, SlackClientSecret,
};
use slack_morphism_reqwest::ReqwestConnector;
use std::time::Duration;
use tracing::Span;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
struct AuthTestResponse {
    ok: bool,
    user: String,
    team: String,
}

fn token() -> SlackApiToken {
    SlackApiToken::new("xoxb-test-token".into())
}

fn connector(server: &MockServer) -> ReqwestConnector {
    ReqwestConnector::with_defaults()
        .expect("default reqwest client")
        .with_slack_api_url(format!("{}/api", server.uri()))
}

async fn run_get<RS>(connector: &ReqwestConnector, method_name: &str) -> ClientResult<RS>
where
    RS: for<'de> serde::de::Deserialize<'de> + Send,
{
    let token = token();
    let span = Span::none();
    let context = SlackClientApiCallContext {
        rate_control_params: None,
        token: Some(&token),
        tracing_span: &span,
        is_sensitive_url: false,
    };
    let url = connector
        .create_method_uri_path(method_name)
        .expect("uri parses");
    connector.http_get_uri(url, context).await
}

#[tokio::test]
async fn get_returns_typed_response_on_ok() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"ok":true,"user":"U12345","team":"T67890"}"#,
            "application/json",
        ))
        .mount(&server)
        .await;

    let connector = connector(&server);
    let response: AuthTestResponse = run_get(&connector, "auth.test").await.unwrap();
    assert!(response.ok);
    assert_eq!(response.user, "U12345");
    assert_eq!(response.team, "T67890");
}

#[tokio::test]
async fn get_maps_slack_error_to_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/auth.test"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(r#"{"ok":false,"error":"invalid_auth"}"#, "application/json"),
        )
        .mount(&server)
        .await;

    let connector = connector(&server);
    let result: ClientResult<AuthTestResponse> = run_get(&connector, "auth.test").await;

    match result {
        Err(SlackClientError::ApiError(err)) => {
            assert_eq!(err.code, "invalid_auth");
            assert!(err.http_response_body.is_some());
        }
        other => panic!("expected ApiError, got {:?}", other),
    }
}

#[tokio::test]
async fn get_maps_429_to_rate_limit_error_with_retry_after() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/auth.test"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "12")
                .set_body_raw(r#"{"ok":false,"error":"ratelimited"}"#, "application/json"),
        )
        .mount(&server)
        .await;

    let connector = connector(&server);
    let result: ClientResult<AuthTestResponse> = run_get(&connector, "auth.test").await;

    match result {
        Err(SlackClientError::RateLimitError(err)) => {
            assert_eq!(err.retry_after, Some(Duration::from_secs(12)));
            assert_eq!(err.code.as_deref(), Some("ratelimited"));
        }
        other => panic!("expected RateLimitError, got {:?}", other),
    }
}

#[tokio::test]
async fn get_maps_429_without_json_to_rate_limit_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/auth.test"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after", "5")
                .set_body_string("Too Many Requests"),
        )
        .mount(&server)
        .await;

    let connector = connector(&server);
    let result: ClientResult<AuthTestResponse> = run_get(&connector, "auth.test").await;

    match result {
        Err(SlackClientError::RateLimitError(err)) => {
            assert_eq!(err.retry_after, Some(Duration::from_secs(5)));
            assert_eq!(err.code, None);
        }
        other => panic!("expected RateLimitError, got {:?}", other),
    }
}

#[tokio::test]
async fn get_maps_5xx_to_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/auth.test"))
        .respond_with(ResponseTemplate::new(503).set_body_string("service unavailable"))
        .mount(&server)
        .await;

    let connector = connector(&server);
    let result: ClientResult<AuthTestResponse> = run_get(&connector, "auth.test").await;

    match result {
        Err(SlackClientError::HttpError(err)) => {
            assert_eq!(err.status_code.as_u16(), 503);
            assert_eq!(
                err.http_response_body.as_deref(),
                Some("service unavailable")
            );
        }
        other => panic!("expected HttpError, got {:?}", other),
    }
}

#[tokio::test]
async fn get_maps_malformed_json_to_protocol_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("not json {", "application/json"))
        .mount(&server)
        .await;

    let connector = connector(&server);
    let result: ClientResult<AuthTestResponse> = run_get(&connector, "auth.test").await;

    match result {
        Err(SlackClientError::ProtocolError(err)) => {
            assert!(err.json_body.is_some());
        }
        other => panic!("expected ProtocolError, got {:?}", other),
    }
}

#[tokio::test]
async fn get_204_returns_empty_object() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/whatever"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;

    let connector = connector(&server);
    let result: ClientResult<serde_json::Value> = run_get(&connector, "whatever").await;
    assert_eq!(result.unwrap(), serde_json::json!({}));
}

#[tokio::test]
async fn post_sends_json_body_and_authorization_header() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/chat.postMessage"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer xoxb-test-token",
        ))
        .and(wiremock::matchers::header(
            "content-type",
            "application/json; charset=utf-8",
        ))
        .and(wiremock::matchers::body_json(serde_json::json!({
            "channel": "C1",
            "text": "hi"
        })))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"ok":true,"channel":"C1","ts":"1.2"}"#,
            "application/json",
        ))
        .mount(&server)
        .await;

    #[derive(Serialize)]
    struct Req {
        channel: &'static str,
        text: &'static str,
    }
    #[derive(Deserialize)]
    struct Resp {
        ok: bool,
        channel: String,
        ts: String,
    }

    let connector = connector(&server);
    let token = token();
    let span = Span::none();
    let context = SlackClientApiCallContext {
        rate_control_params: None,
        token: Some(&token),
        tracing_span: &span,
        is_sensitive_url: false,
    };
    let url = connector
        .create_method_uri_path("chat.postMessage")
        .unwrap();
    let body = Req {
        channel: "C1",
        text: "hi",
    };
    let resp: Resp = connector.http_post_uri(url, &body, context).await.unwrap();
    assert!(resp.ok);
    assert_eq!(resp.channel, "C1");
    assert_eq!(resp.ts, "1.2");
}

#[tokio::test]
async fn client_session_round_trip_through_connector() {
    // Full SlackClient → session → typed call path, exercising
    // slack-morphism's wrapper layer on top of our connector.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/auth.test"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(
                r#"{"ok":true,"url":"https://example.slack.com/","team":"team","user":"u","team_id":"T1","user_id":"U1"}"#,
                "application/json",
            ),
        )
        .mount(&server)
        .await;

    let connector = ReqwestConnector::with_defaults()
        .unwrap()
        .with_slack_api_url(format!("{}/api", server.uri()));
    let client = SlackClient::new(connector);
    let token = token();
    let session = client.open_session(&token);
    let response = session.auth_test().await.expect("auth_test ok");
    assert_eq!(response.user_id.value(), "U1");
}

#[tokio::test]
async fn multipart_uploads_file_and_form_fields() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/files.upload"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer xoxb-test-token",
        ))
        .and(wiremock::matchers::header_regex(
            "content-type",
            r"^multipart/form-data; boundary=",
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(r#"{"ok":true,"file":{"id":"F1"}}"#, "application/json"),
        )
        .mount(&server)
        .await;

    #[derive(Deserialize)]
    struct Resp {
        ok: bool,
    }

    let connector = connector(&server);
    let token = token();
    let span = Span::none();
    let context = SlackClientApiCallContext {
        rate_control_params: None,
        token: Some(&token),
        tracing_span: &span,
        is_sensitive_url: false,
    };
    let url = connector
        .create_method_uri_path("files.upload")
        .expect("uri parses");

    let file_bytes: &[u8] = b"hello world";
    let file = FileMultipartData {
        name: "hello.txt".into(),
        content_type: "text/plain".into(),
        data: file_bytes,
    };
    let params: Vec<(&str, Option<&str>)> = vec![
        ("channels", Some("C1")),
        ("title", Some("hello")),
        ("filetype", None),
    ];

    let resp: Resp = connector
        .http_post_uri_multipart_form(url, Some(file), &params, context)
        .await
        .expect("multipart post ok");
    assert!(resp.ok);
}

#[tokio::test]
async fn multipart_without_file_still_sends_form_fields() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/files.completeUploadExternal"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"ok":true}"#, "application/json"))
        .mount(&server)
        .await;

    #[derive(Deserialize)]
    struct Resp {
        ok: bool,
    }

    let connector = connector(&server);
    let token = token();
    let span = Span::none();
    let context = SlackClientApiCallContext {
        rate_control_params: None,
        token: Some(&token),
        tracing_span: &span,
        is_sensitive_url: false,
    };
    let url = connector
        .create_method_uri_path("files.completeUploadExternal")
        .unwrap();
    let params: Vec<(&str, Option<&str>)> = vec![("files", Some("[{\"id\":\"F1\"}]"))];
    let resp: Resp = connector
        .http_post_uri_multipart_form::<_, _, &str>(url, None, &params, context)
        .await
        .expect("ok");
    assert!(resp.ok);
}

#[tokio::test]
async fn binary_post_passes_content_type_and_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/files.upload.v2"))
        .and(wiremock::matchers::header("content-type", "image/png"))
        .and(wiremock::matchers::header(
            "authorization",
            "Bearer xoxb-test-token",
        ))
        .and(wiremock::matchers::body_bytes(b"\x89PNG\r\n".as_ref()))
        .respond_with(ResponseTemplate::new(200).set_body_raw(r#"{"ok":true}"#, "application/json"))
        .mount(&server)
        .await;

    #[derive(Deserialize)]
    struct Resp {
        ok: bool,
    }

    let connector = connector(&server);
    let token = token();
    let span = Span::none();
    let context = SlackClientApiCallContext {
        rate_control_params: None,
        token: Some(&token),
        tracing_span: &span,
        is_sensitive_url: true,
    };
    let url = connector.create_method_uri_path("files.upload.v2").unwrap();
    let body: &[u8] = b"\x89PNG\r\n";
    let resp: Resp = connector
        .http_post_uri_binary::<_>(url, "image/png".to_string(), body, context)
        .await
        .expect("binary upload ok");
    assert!(resp.ok);
}

#[tokio::test]
async fn oauth_endpoint_uses_basic_auth_not_bearer() {
    let server = MockServer::start().await;
    // The full URL we'll pass to http_get_with_client_secret. For
    // production this is `https://slack.com/api/oauth.v2.access`; the
    // mock just needs a deterministic path to match on.
    Mock::given(method("GET"))
        .and(path("/api/oauth.v2.access"))
        // Basic Auth: "id:secret" base64-encoded → "aWQ6c2VjcmV0"
        .and(wiremock::matchers::header(
            "authorization",
            "Basic aWQ6c2VjcmV0",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"ok":true,"access_token":"xoxb-issued"}"#,
            "application/json",
        ))
        .mount(&server)
        .await;

    #[derive(Deserialize)]
    struct Resp {
        ok: bool,
        access_token: String,
    }

    let connector = connector(&server);
    let url = connector.create_method_uri_path("oauth.v2.access").unwrap();
    let client_id: SlackClientId = "id".to_string().into();
    let client_secret: SlackClientSecret = "secret".to_string().into();
    let resp: Resp = connector
        .http_get_with_client_secret(url, &client_id, &client_secret)
        .await
        .expect("oauth call ok");
    assert!(resp.ok);
    assert_eq!(resp.access_token, "xoxb-issued");
}

#[tokio::test]
async fn with_slack_api_url_routes_to_overridden_host() {
    // Two parallel mock servers — the connector should hit the one
    // we override to, not the default slack.com (which would fail to
    // resolve in a test env anyway).
    let primary = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/auth.test"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"ok":true,"user":"PRIMARY","team":"T"}"#,
            "application/json",
        ))
        .mount(&primary)
        .await;

    // Build a connector pointed at the primary; the integration tests
    // already exercise this implicitly, but make the override
    // explicitly load-bearing here.
    let connector = ReqwestConnector::with_defaults()
        .unwrap()
        .with_slack_api_url(format!("{}/api", primary.uri()));
    let resp: AuthTestResponse = run_get(&connector, "auth.test").await.unwrap();
    assert_eq!(resp.user, "PRIMARY");
}

#[tokio::test]
async fn non_json_200_falls_back_to_empty_object() {
    // 200 response with a non-JSON content-type returns parsed `{}`
    // (matches upstream hyper connector). This exercises the
    // `200 | 204` branch via a text/plain body.
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/whatever"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let connector = connector(&server);
    let resp: ClientResult<serde_json::Value> = run_get(&connector, "whatever").await;
    assert_eq!(resp.unwrap(), serde_json::json!({}));
}

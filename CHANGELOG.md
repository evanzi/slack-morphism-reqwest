# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - TBD

Initial release.

### Added

- `ReqwestConnector` implementation of `slack_morphism::SlackClientHttpConnector`
- `ReqwestConnector::new` for callers who supply their own `reqwest::Client`
- `ReqwestConnector::with_defaults` for a sensible default client (30s timeout, rustls TLS)
- `ReqwestConnector::with_slack_api_url` for tests and proxied environments
- All four `SlackClientHttpConnector` trait methods implemented:
  GET, POST, multipart form, and binary upload
- `http_get_with_client_secret` for OAuth client_id/client_secret flow
- Status code routing matching the official hyper connector:
  200 OK JSON, 200/204 empty, 429 with retry-after, all other statuses
- Response body redaction for sensitive URLs in tracing spans

[Unreleased]: https://github.com/evanzi/slack-morphism-reqwest/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/evanzi/slack-morphism-reqwest/releases/tag/v0.1.0

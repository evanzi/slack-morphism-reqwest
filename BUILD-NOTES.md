# Build Notes — slack-morphism-reqwest v0.1.0

Pre-publish handoff notes. Not part of the published crate (excluded by `include` allowlist in Cargo.toml).

## What's done

**Crate scaffolded and ready for publication.**

- `Cargo.toml` — full crates.io metadata (license, keywords, categories, repository, MSRV 1.75)
- `src/lib.rs` — `ReqwestConnector` impl of `SlackClientHttpConnector` (all 5 trait methods)
- `tests/connector.rs` — 15 wiremock integration tests + 1 doctest, all passing
- `examples/post_message.rs` — runnable end-to-end example
- `README.md`, `CHANGELOG.md`, `LICENSE-MIT`, `LICENSE-APACHE`, `.gitignore`
- `.github/workflows/ci.yml` — fmt, clippy (`-D warnings`), test on stable + MSRV, doc with `RUSTDOCFLAGS=-D warnings`, cargo-audit
- Local git repo initialized + initial commit

**All verification gates pass:**

```
cargo fmt --all -- --check    OK
cargo clippy --all-targets -- -D warnings    OK
cargo test     15/15 integration + 1/1 doctest
cargo doc --no-deps    OK with -D warnings
cargo publish --dry-run    OK
cargo audit    OK (no advisories)
```

## Reviews completed

Both ran in parallel before commit. Major findings addressed inline:

**From architecture review (5 must-fixes, all addressed):**
- Fixed `with_defaults()` doc/impl mismatch: gzip is now actually enabled, doc rewritten as bulleted list with accurate claims
- Pinned slack-morphism with explicit caret semantics (kept `"2.20"` since cargo treats it as `^2.20` = >=2.20, <3.0)
- Test gaps closed: added multipart, binary, OAuth, and override-URL tests
- Documented tier-aware throttling absence prominently in README's "Behavior" section and the struct doc
- Added `#![forbid(unsafe_code)]`

**From code review (3 must-fixes + 2 should-fixes, all addressed):**
- Added `examples/**/*.rs` to the published `include` list
- Added 5 new tests (multipart with file, multipart without file, binary upload, OAuth client_secret flow, custom slack_api_url override)
- Fixed `is_json` parity bug — missing/unparseable `Content-Type` now treated as JSON (matching upstream's `iter().all(...)` semantics)
- Added `'p` lifetime to `http_post_uri_binary` for trait/upstream signature parity (with PhantomData stub)
- Removed misleading test (`missing_content_type_is_treated_as_json`) replaced with `non_json_200_falls_back_to_empty_object` that actually exercises the documented branch
- Added `From<reqwest::Client> for ReqwestConnector` for ergonomic conversion

## What you do when you get back

In this order:

### 1. Create the public GitHub repo

```bash
cd ~/Projects/slack-morphism-reqwest
gh repo create slack-morphism-reqwest --public \
  --source . \
  --description "A reqwest-based HTTP connector for slack-morphism" \
  --homepage "https://crates.io/crates/slack-morphism-reqwest"
git push -u origin main
```

(The `gh repo create` is the first action that's visible to the world. I held off so you could review the code first.)

### 2. Verify CI runs green on GitHub Actions

After the push, watch `gh run list --branch main` and confirm the `fmt`, `clippy`, `test`, `doc`, and `audit` jobs all pass on the first run. Fix anything CI catches that local didn't.

### 3. Update `CHANGELOG.md` with the publish date

```diff
- ## [0.1.0] - TBD
+ ## [0.1.0] - 2026-MM-DD
```

Commit + push.

### 4. Tag and publish to crates.io

```bash
git tag v0.1.0
git push origin v0.1.0
cargo publish
```

**Heads up:** `cargo publish` is irreversible — once `slack-morphism-reqwest@0.1.0` is on crates.io, that name+version slot is taken forever. The `--dry-run` already succeeded so this should go through cleanly.

You'll need to be logged in: `cargo login <token>` from your crates.io account settings.

### 5. (Optional) Open an issue on upstream slack-morphism

Once published, consider opening a friendly issue on `abdolence/slack-morphism-rust` to register the new connector as an alternative. Suggested wording:

> Hi! I've published a third-party reqwest-based connector for slack-morphism at `slack-morphism-reqwest` (https://crates.io/crates/slack-morphism-reqwest). Posting in case you'd like to mention it in your README as an option for projects that already use reqwest. Happy to submit a PR adding it to the docs if that's helpful.

## Known limitations / v0.2 candidates

These were noted by the reviewers but deferred from v0.1:

- **TLS feature flags.** Currently hardcodes `rustls-tls` on the connector's reqwest dep. v0.2 should expose `rustls-tls` (default) vs `native-tls` as crate-level features. Adding features is non-breaking, so this is safe to defer.
- **Tier-aware proactive throttling.** Upstream's hyper connector ships `SlackTokioRateController`; we don't. Document is in place; if users ask, port the throttler.
- **Dependabot config.** Plan called for `.github/dependabot.yml`; punting to v0.2.
- **Windows CI row.** Linux + macOS in CI matrix; Windows is a cheap addition.
- **Scheduled (weekly) cargo-audit cron.** Currently runs on push/PR only.
- **`map_reqwest_error` simplification.** Architecture review flagged the three-layer error wrapping in the `is_decode()` branch as theater. Since gzip isn't enabled in `with_defaults`, this branch rarely fires. Worth simplifying in v0.2.
- **Builder pattern for `ReqwestConnector`.** If we add 3+ more options (e.g., custom rate-limit policy, request middleware, default headers), introduce a `ReqwestConnectorBuilder`. Not worth it for the current 1-2 options.
- **Stable redaction hash for log correlation.** `redacted_url` uses `DefaultHasher` which is non-deterministic across runs; if anyone wants log correlation, swap to a stable hasher.
- **`pub fn http()`.** Code review questioned whether this is forward-compat-safe. Kept for v0.1 since callers may legitimately want the underlying client. Re-evaluate if we ever need `Arc<reqwest::Client>` semantics.

## Files in the local working tree

```
slack-morphism-reqwest/
├── BUILD-NOTES.md          (this file — excluded from publish)
├── CHANGELOG.md
├── Cargo.lock              (gitignored)
├── Cargo.toml
├── LICENSE-APACHE
├── LICENSE-MIT
├── README.md
├── .gitignore
├── .github/
│   └── workflows/
│       └── ci.yml
├── examples/
│   └── post_message.rs
├── src/
│   └── lib.rs
└── tests/
    └── connector.rs
```

## Compatibility matrix (as published)

| `slack-morphism-reqwest` | `slack-morphism` | `reqwest`   | MSRV   |
|--------------------------|------------------|-------------|--------|
| `0.1.0`                  | `2.20.x` (incl. forward `^2`) | `0.12.x` | 1.88 |

## Funes integration plan

Once `slack-morphism-reqwest@0.1.0` is published, Funes can adopt it per Phase 2 of `Funes-Slack-Morphism-Migration-Plan.md`:

```toml
# crates/slack/Cargo.toml additions
slack-morphism = { version = "2.20", default-features = false }
slack-morphism-reqwest = "0.1"
```

That migration is conditional on Open Question #1 in the plan being answered (whether Socket Mode delivers user-scoped events). Don't start it until that's verified.

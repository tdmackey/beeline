# Foursquare API Validation

This crate implements the current Foursquare v2 user-auth flow and Swarm checkin endpoint documented at:

- https://docs.foursquare.com/developer/reference/v2-authentication
- https://docs.foursquare.com/developer/reference/get-user-checkins
- https://docs.foursquare.com/developer/reference/v2-rate-limits
- https://docs.foursquare.com/developer/reference/v2-errors

## Implemented Wire Contract

- OAuth authorize URL: `https://foursquare.com/oauth2/authenticate`
- OAuth token exchange URL: `https://foursquare.com/oauth2/access_token`
- OAuth token exchange query params: `client_id`, `client_secret`, `grant_type=authorization_code`, `redirect_uri`, `code`
- User auth header: `Authorization: Bearer <ACCESS_TOKEN>`
- Latest checkins endpoint: `GET https://api.foursquare.com/v2/users/self/checkins`
- Checkins query params: `v`, `limit`, `offset`
- Rate-limit headers read: `X-RateLimit-Limit`, `X-RateLimit-Remaining`, `X-RateLimit-Reset`

## Local Validation

Run deterministic mock-backed contract tests:

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

These tests validate the HTTP method, path, query params, bearer token header, OAuth exchange params, response parsing, and partial failures without hitting Foursquare.

## Public Crate Gate

Install the dependency review tools once:

```powershell
cargo install cargo-audit cargo-deny
```

Then run the full local release gate before publishing or cutting a Discord bot integration PR:

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo audit
cargo deny check
cargo publish --dry-run --allow-dirty
```

`cargo audit` checks `Cargo.lock` against RustSec advisories. `cargo deny check` enforces the repository dependency policy in `deny.toml`: advisories, license allowlist, duplicate dependency warnings, and source registry restrictions.

## Live API Validation

This crate intentionally does not ship credential-driven live tests. OAuth authorization codes, client secrets, and bearer tokens are sensitive and do not belong in a public crate test workflow.

Applications should validate their own deployed OAuth callback flow against their registered Foursquare project. The crate-level quality gate stays deterministic and mock-backed so CI never depends on external credentials, real user data, or Foursquare availability.

## Known Limit

The current Foursquare checkins reference page documents the endpoint and required query params, but it does not publish a detailed response schema. The crate therefore keeps response model fields optional and preserves unknown fields in `extra` maps so it can tolerate undocumented response changes.

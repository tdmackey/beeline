# Security Policy

## Supported Versions

This crate is pre-1.0. Security fixes should target the current `main` branch until a version support policy exists.

## Reporting Vulnerabilities

Do not open a public issue for credential leaks, token handling bugs, OAuth state validation bypasses, or request-signing/authentication flaws. Report them through the repository owner's preferred private channel.

## Maintainer Checklist

Before publishing a crate release or integrating a new version into a bot:

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo audit
cargo deny check
cargo publish --dry-run --allow-dirty
```

## OAuth Safety Notes

- Treat Foursquare client secrets, OAuth authorization codes, and access tokens as secrets.
- Authorization codes are single-use and should not be logged.
- `LinkState` is HMAC-signed caller-visible OAuth state. The bot must still validate the nonce against server-side pending state before linking a Discord user, so a captured callback cannot be replayed.
- The bot owns token storage and should encrypt or otherwise protect persisted access tokens according to its deployment environment.

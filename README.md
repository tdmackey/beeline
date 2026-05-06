# Beeline

Rust helpers for Foursquare/Swarm OAuth and latest-checkin polling.

This crate is designed for Discord bots and other Rust services, but it does not depend on Discord, Poise, Serenity, Rusqlite, or any application internals.

## Docs

- [Project page](https://tdmackey.github.io/beeline/)
- [Discord bot integration guide](docs/discord-bot-integration.md)
- [Foursquare API validation](docs/api-validation.md)
- [Security policy](SECURITY.md)

## Quick Validation

```powershell
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo audit
cargo deny check
cargo publish --dry-run --allow-dirty
```

`cargo audit` and `cargo deny` are release-gate tools for public crate hygiene:

```powershell
cargo install cargo-audit cargo-deny
```

GitHub Actions runs the same gate on pull requests and `main`: MSRV check, format, clippy, docs with warnings denied, cross-platform tests, dependency audit/policy, and publish dry-run. Dependabot is configured for Cargo and GitHub Actions updates.

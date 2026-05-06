# Discord Bot Integration Guide

This guide describes how a Rust Discord bot can use `beeline` to link Discord users to Foursquare/Swarm accounts and poll their latest checkins.

## What Beeline Owns

`beeline` is intentionally storage- and Discord-framework-neutral. It owns:

- Foursquare OAuth URL generation.
- OAuth callback parsing.
- OAuth code exchange.
- Discord-friendly signed `LinkState` encoding/decoding with plain `u64` IDs.
- `GET /v2/users/self/checkins` polling.
- Multi-user polling with bounded concurrency.
- Foursquare response models, rate-limit metadata, and typed errors.

The bot application should own:

- Slash commands and Discord messaging.
- The callback HTTP server or manual callback command.
- SQLite migrations and token storage.
- Pending OAuth nonce storage.
- Poll scheduling and channel posting policy.
- Secret management.

## Dependency

In the bot's `Cargo.toml`, add `beeline` as a path dependency while developing locally:

```toml
beeline = { path = "../beeline" }
```

Adjust the path if the bot repository and `beeline` are not siblings.

## Config

Add a `swarm` section to the bot config:

```json
{
  "swarm": {
    "client_id": "FOURSQUARE_CLIENT_ID",
    "client_secret": "FOURSQUARE_CLIENT_SECRET",
    "redirect_uri": "https://swarm-dev.example.com/swarm/callback",
    "state_signing_key": "LONG_RANDOM_SECRET_FOR_OAUTH_STATE",
    "poll_interval_seconds": 300,
    "poll_limit": 10,
    "max_concurrency": 4,
    "enabled": true
  }
}
```

Recommended Rust config shape:

```rust
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct SwarmConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default)]
    pub redirect_uri: Option<String>,
    #[serde(default)]
    pub state_signing_key: Option<String>,
    #[serde(default = "default_swarm_poll_interval")]
    pub poll_interval_seconds: u64,
    #[serde(default = "default_swarm_poll_limit")]
    pub poll_limit: u32,
    #[serde(default = "default_swarm_max_concurrency")]
    pub max_concurrency: usize,
}
```

Keep `client_secret`, `state_signing_key`, authorization codes, and access tokens out of logs. Rotate any secret that has been pasted into chat, committed, or logged.

## SQLite Storage

Use application-owned migrations, not Beeline, for persistence.

Suggested linked-account table:

```sql
CREATE TABLE IF NOT EXISTS swarm_accounts (
    discord_user_id INTEGER PRIMARY KEY,
    access_token TEXT NOT NULL,
    foursquare_user_id TEXT,
    display_name TEXT,
    last_seen_checkin_id TEXT,
    last_seen_created_at INTEGER,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    revoked_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_swarm_accounts_enabled
    ON swarm_accounts(enabled);
```

Suggested pending-OAuth table:

```sql
CREATE TABLE IF NOT EXISTS swarm_oauth_states (
    nonce TEXT PRIMARY KEY,
    discord_user_id INTEGER NOT NULL,
    guild_id INTEGER,
    channel_id INTEGER,
    issued_at INTEGER NOT NULL,
    consumed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_swarm_oauth_states_user
    ON swarm_oauth_states(discord_user_id);
```

The pending table lets the bot reject stale, reused, or mismatched OAuth callbacks.

## Slash Commands

Recommended v1 commands:

- `/swarm link`: starts OAuth linking for the caller.
- `/swarm status`: shows whether the caller is linked.
- `/swarm unlink`: disables or deletes the caller's token.
- `/swarm checkins`: manually fetches and shows the caller's latest checkins.
- `/swarm poll on/off`: owner/admin command to enable posting in a channel, if the bot should post automatically.

The library does not enforce command names.

## Link Flow

For `/swarm link`:

1. Generate a random nonce.
2. Store the nonce in `swarm_oauth_states` with `discord_user_id`, optional `guild_id`, optional `channel_id`, and `issued_at`.
3. Build a `beeline::LinkState`:

```rust
let state = beeline::LinkState::new(discord_user_id, nonce)
    .with_guild_id(guild_id)
    .with_channel_id(channel_id);
```

4. Encode the state and build the authorization URL:

```rust
let oauth = beeline::OAuthConfig::new(client_id, client_secret, redirect_uri)?;
let state_signing_key = config.swarm.state_signing_key.as_deref()
    .ok_or("missing swarm.state_signing_key")?;
let url = swarm_client.authorization_url(
    &oauth,
    beeline::AuthorizationRequest::new().with_state(state.encode(state_signing_key.as_bytes())?),
);
```

5. Send the URL to the user. Prefer an ephemeral slash-command response or DM if possible.

## Callback Flow

The bot can support either a real HTTP callback route or a manual paste command. The HTTP callback is better.

For `GET /swarm/callback?code=...&state=...`:

1. Parse the callback:

```rust
let callback = beeline::parse_authorization_callback(full_callback_url)?;
```

2. Decode and validate state:

```rust
let encoded_state = callback.state.as_deref().ok_or("missing state")?;
let state_signing_key = config.swarm.state_signing_key.as_deref()
    .ok_or("missing swarm.state_signing_key")?;
let state = beeline::LinkState::decode_with_max_age(
    encoded_state,
    state_signing_key.as_bytes(),
    std::time::Duration::from_secs(15 * 60),
)?;
```

3. Look up `state.nonce` in `swarm_oauth_states`.
4. Verify it is unconsumed and belongs to `state.discord_user_id`.
5. Exchange the code:

```rust
let token = swarm_client.exchange_code(&oauth, callback.code, None).await?;
```

6. Store or update `swarm_accounts.access_token`.
7. Mark the nonce consumed.
8. Show a simple browser response and optionally notify the Discord user/channel.

OAuth codes are single-use. Do not run multiple exchange attempts with the same code.

## Polling Flow

Use `latest_checkins_for_users` for scheduled polling:

```rust
let users = linked_accounts.into_iter().map(|account| beeline::AuthorizedUser {
    external_user_id: account.discord_user_id,
    access_token: account.access_token,
});

let results = swarm_client
    .latest_checkins_for_users(
        users,
        beeline::PollOptions {
            max_concurrency: config.swarm.max_concurrency,
            query: beeline::CheckinsQuery {
                limit: config.swarm.poll_limit,
                offset: 0,
                version: None,
            },
        },
    )
    .await;
```

For each successful user result:

- Sort returned checkins oldest-to-newest before posting.
- Skip checkins at or before `last_seen_created_at` / `last_seen_checkin_id`.
- Post only new checkins.
- Update `last_seen_checkin_id` and `last_seen_created_at` after successful posting.

For failures:

- `Error::InvalidAuth`: mark the account revoked or disabled and ask the user to relink.
- `Error::Forbidden`: treat as revoked/privacy denied unless it later succeeds.
- `Error::RateLimited`: back off until `reset_at` if present.
- `Error::QuotaExceeded`: stop polling until the next day or operator action.
- `Error::Server` and request failures: retry later without disabling the account.

## Rate Limits And Cost

Foursquare's current docs say authenticated v2 requests are limited to 500 requests per hour per OAuth token. The pricing docs currently say checkins, lists, tastes, tips, and users endpoints remain free.

Practical defaults:

- Poll every 5 minutes per user: 12 requests/hour/token.
- `limit = 10` or `limit = 20` for normal polling.
- Use a higher limit only for manual backfill or first sync.

Do not enrich every checkin with paid venue endpoints until you decide on a budget. Checkin polling itself is the free part.

## OAuth Testing

Test the OAuth flow in the bot application, not inside this library crate. The bot owns its callback route, pending-state storage, token persistence, and user messaging, so end-to-end OAuth tests should live beside that code.

For crate-level validation, `beeline` uses deterministic mock-backed tests that verify URL construction, callback parsing, code exchange shape, bearer token usage, checkin parsing, per-user partial failures, and error mapping without handling real credentials or user data.

## Discord Bot Implementation Checklist

1. Add the `beeline` dependency.
2. Add `SwarmConfig` to the bot's config loading.
3. Add migrations for `swarm_accounts` and `swarm_oauth_states`.
4. Add a `SwarmManager` around SQLite account/state operations.
5. Add `/swarm link`, `/swarm status`, `/swarm unlink`, and `/swarm checkins`.
6. Add an HTTP callback route or a temporary manual callback command.
7. Add a scheduler that polls enabled linked accounts.
8. Add dedupe using `last_seen_checkin_id` and `last_seen_created_at`.
9. Add rate-limit-aware backoff.
10. Add user-facing relink handling for revoked tokens.
11. Rotate any client secret that has been pasted into chat/logs.

## Acceptance Criteria

The integration is ready when:

- A Discord user can run `/swarm link`, approve Foursquare, and become linked.
- `/swarm status` shows linked state.
- `/swarm checkins` fetches that user's latest checkins.
- Scheduled polling posts only new checkins.
- Re-running the poll does not repost already seen checkins.
- Revoked/invalid tokens disable the link and tell the user to relink.
- Normal tests pass without live credentials.
- The bot's own callback integration test passes against its registered Foursquare redirect URL.

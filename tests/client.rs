use beeline::{
    AccessToken, AuthorizationRequest, AuthorizedUser, CheckinsQuery, Error, LinkState,
    OAuthConfig, PollOptions, SwarmClient, parse_authorization_callback,
};
mod common;
use common::{MockResponse, MockServer};
use std::collections::{BTreeMap, HashMap};
use std::time::Duration;
use url::Url;

const STATE_SIGNING_KEY: &[u8] = b"test-state-signing-key";
const DEFAULT_OAUTH_AUTHORIZE_URL: &str = "https://foursquare.com/oauth2/authenticate";
const DEFAULT_USER_AGENT: &str = concat!("beeline/", env!("CARGO_PKG_VERSION"));
const MAX_RESPONSE_BODY_BYTES: usize = 1024 * 1024;

#[test]
fn authorization_url_contains_oauth_parameters_and_state() {
    let client = SwarmClient::new();
    let config = OAuthConfig::new(
        "client-id",
        "client-secret",
        "https://bot.example.com/swarm/callback",
    )
    .unwrap();
    let state = LinkState::new(42, "nonce-123")
        .with_guild_id(7)
        .with_channel_id(9)
        .encode(STATE_SIGNING_KEY)
        .unwrap();

    let url = client.authorization_url(
        &config,
        AuthorizationRequest::new().with_state(state.clone()),
    );

    let pairs: HashMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(
        url.as_str().split('?').next().unwrap(),
        DEFAULT_OAUTH_AUTHORIZE_URL
    );
    assert_eq!(pairs.get("client_id"), Some(&"client-id".to_string()));
    assert_eq!(pairs.get("response_type"), Some(&"code".to_string()));
    assert_eq!(
        pairs.get("redirect_uri"),
        Some(&"https://bot.example.com/swarm/callback".to_string())
    );
    assert_eq!(pairs.get("state"), Some(&state));
}

#[test]
fn parses_authorization_callback() {
    let callback = parse_authorization_callback(
        "https://bot.example.com/swarm/callback?code=abc123&state=state123",
    )
    .unwrap();

    assert_eq!(callback.code, "abc123");
    assert_eq!(callback.state.as_deref(), Some("state123"));
}

#[test]
fn link_state_round_trips_and_rejects_bad_or_expired_values() {
    let state = LinkState::new(123, "nonce")
        .with_guild_id(456)
        .with_channel_id(789);
    let encoded = state.encode(STATE_SIGNING_KEY).unwrap();
    assert_eq!(
        LinkState::decode(&encoded, STATE_SIGNING_KEY).unwrap(),
        state
    );

    assert!(matches!(
        LinkState::decode("not-valid-state", STATE_SIGNING_KEY),
        Err(Error::InvalidState(_))
    ));

    let mut forged = encoded.clone().into_bytes();
    let last = forged.last_mut().unwrap();
    *last = if *last == b'A' { b'B' } else { b'A' };
    let forged = String::from_utf8(forged).unwrap();
    assert!(matches!(
        LinkState::decode(forged, STATE_SIGNING_KEY),
        Err(Error::InvalidState(_))
    ));

    let expired = LinkState::new(123, "nonce")
        .with_issued_at(1)
        .encode(STATE_SIGNING_KEY)
        .unwrap();
    assert!(matches!(
        LinkState::decode_with_max_age(expired, STATE_SIGNING_KEY, Duration::from_secs(60)),
        Err(Error::ExpiredState { .. })
    ));

    let future = LinkState::new(123, "nonce")
        .with_issued_at(chrono::Utc::now().timestamp() + 10 * 60)
        .encode(STATE_SIGNING_KEY)
        .unwrap();
    assert!(matches!(
        LinkState::decode_with_max_age(future, STATE_SIGNING_KEY, Duration::from_secs(60)),
        Err(Error::InvalidState(_))
    ));
}

#[test]
fn debug_redacts_secret_bearing_types() {
    let oauth = OAuthConfig::new(
        "client",
        "client-secret",
        "https://bot.example.com/callback",
    )
    .unwrap();
    let mut extra = BTreeMap::new();
    extra.insert(
        "refresh_token".to_string(),
        serde_json::Value::String("refresh-secret".to_string()),
    );
    let token = AccessToken {
        access_token: "access-token".to_string(),
        token_type: Some("bearer".to_string()),
        scope: None,
        extra,
    };
    let user = AuthorizedUser {
        external_user_id: 42_u64,
        access_token: "user-token".to_string(),
    };

    let rendered = format!("{oauth:?} {token:?} {user:?}");

    assert!(!rendered.contains("client-secret"));
    assert!(!rendered.contains("access-token"));
    assert!(!rendered.contains("refresh-secret"));
    assert!(!rendered.contains("user-token"));
    assert!(rendered.contains("refresh_token"));
    assert!(rendered.contains("<redacted>"));
}

#[test]
fn rejects_insecure_endpoints_without_test_opt_in() {
    let err = SwarmClient::builder()
        .api_base_url(Url::parse("http://127.0.0.1:1234/v2/").unwrap())
        .build()
        .unwrap_err();

    assert!(matches!(
        err,
        Error::InsecureEndpoint {
            name: "api_base_url",
            ..
        }
    ));
}

#[tokio::test]
async fn exchanges_authorization_code_for_access_token() {
    let server = MockServer::spawn(vec![MockResponse::json(
        200,
        vec![],
        r#"{"access_token":"access-token","token_type":"bearer"}"#,
    )])
    .await;
    let client = test_client(&server);
    let config = OAuthConfig::new("client", "secret", "https://bot.example.com/callback").unwrap();

    let token = client
        .exchange_code(&config, "code-123", None)
        .await
        .unwrap();

    assert_eq!(token.access_token, "access-token");
    let requests = server.requests().await;
    assert_eq!(requests.len(), 1);
    assert!(requests[0].contains("GET /oauth2/access_token?"));
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains(&format!("user-agent: {DEFAULT_USER_AGENT}"))
    );
    assert!(requests[0].contains("client_id=client"));
    assert!(requests[0].contains("client_secret=secret"));
    assert!(requests[0].contains("grant_type=authorization_code"));
    assert!(requests[0].contains("code=code-123"));
}

#[tokio::test]
async fn fetches_latest_checkins_with_bearer_token() {
    let server = MockServer::spawn(vec![MockResponse::json(
        200,
        vec![
            ("X-RateLimit-Limit", "500"),
            ("X-RateLimit-Remaining", "499"),
        ],
        r#"{
            "meta": {"code": 200, "requestId": "req-1"},
            "response": {
                "checkins": {
                    "count": 1,
                    "items": [
                        {
                            "id": "checkin-1",
                            "createdAt": 1710000000,
                            "venue": {
                                "id": "venue-1",
                                "name": "Coffee Shop",
                                "location": {"city": "Seattle", "lat": 47.6, "lng": -122.3}
                            }
                        }
                    ]
                }
            }
        }"#,
    )])
    .await;
    let client = test_client(&server);

    let result = client
        .latest_checkins(
            "user-token",
            CheckinsQuery {
                limit: 10,
                offset: 5,
                version: Some("20240101".to_string()),
            },
        )
        .await
        .unwrap();

    assert_eq!(result.rate_limit.limit, Some(500));
    assert_eq!(result.rate_limit.remaining, Some(499));
    assert_eq!(result.checkins.count, Some(1));
    assert_eq!(result.checkins.items[0].id.as_deref(), Some("checkin-1"));
    assert_eq!(
        result.checkins.items[0]
            .venue
            .as_ref()
            .and_then(|venue| venue.name.as_deref()),
        Some("Coffee Shop")
    );

    let requests = server.requests().await;
    let raw = requests[0].to_ascii_lowercase();
    assert!(raw.contains("get /v2/users/self/checkins?"));
    assert!(raw.contains("authorization: bearer user-token"));
    assert!(raw.contains("v=20240101"));
    assert!(raw.contains("limit=10"));
    assert!(raw.contains("offset=5"));
}

#[tokio::test]
async fn normalizes_api_base_url_as_directory() {
    let server = MockServer::spawn(vec![MockResponse::json(
        200,
        vec![],
        r#"{"meta":{"code":200},"response":{"checkins":{"count":0,"items":[]}}}"#,
    )])
    .await;
    let client = SwarmClient::builder()
        .api_base_url(server.url("/v2"))
        .oauth_authorize_url(server.url("/oauth2/authenticate"))
        .oauth_access_token_url(server.url("/oauth2/access_token"))
        .danger_accept_insecure_http_for_tests(true)
        .build()
        .unwrap();

    client
        .latest_checkins("token", CheckinsQuery::default())
        .await
        .unwrap();

    let requests = server.requests().await;
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("get /v2/users/self/checkins?")
    );
}

#[tokio::test]
async fn polls_multiple_users_with_partial_failures() {
    let server = MockServer::spawn(vec![
        MockResponse::json(
            200,
            vec![],
            r#"{"meta":{"code":200},"response":{"checkins":{"count":0,"items":[]}}}"#,
        ),
        MockResponse::json(
            401,
            vec![],
            r#"{"meta":{"code":401,"errorType":"invalid_auth","errorDetail":"bad token"}}"#,
        ),
    ])
    .await;
    let client = test_client(&server);
    let users = vec![
        AuthorizedUser {
            external_user_id: 111_u64,
            access_token: "good-token".to_string(),
        },
        AuthorizedUser {
            external_user_id: 222_u64,
            access_token: "bad-token".to_string(),
        },
    ];

    let results = client
        .latest_checkins_for_users(
            users,
            PollOptions {
                max_concurrency: 1,
                ..PollOptions::default()
            },
        )
        .await;

    assert!(results.get(&111).unwrap().is_ok());
    assert!(matches!(
        results.get(&222).unwrap(),
        Err(Error::InvalidAuth { .. })
    ));

    let requests = server.requests().await;
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("authorization: bearer good-token")
    );
    assert!(
        requests[1]
            .to_ascii_lowercase()
            .contains("authorization: bearer bad-token")
    );
}

#[tokio::test]
async fn maps_foursquare_error_responses() {
    assert_error_response(
        MockResponse::json(
            400,
            vec![],
            r#"{"meta":{"code":400,"errorType":"param_error"}}"#,
        ),
        |err| matches!(err, Error::BadRequest { .. }),
    )
    .await;
    assert_error_response(
        MockResponse::json(
            401,
            vec![],
            r#"{"meta":{"code":401,"errorType":"invalid_auth"}}"#,
        ),
        |err| matches!(err, Error::InvalidAuth { .. }),
    )
    .await;
    assert_error_response(
        MockResponse::json(403, vec![("X-RateLimit-Reset", "1710001234")], ""),
        |err| {
            matches!(
                err,
                Error::RateLimited {
                    reset_at: Some(1710001234),
                    ..
                }
            )
        },
    )
    .await;
    assert_error_response(
        MockResponse::json(
            429,
            vec![],
            r#"{"meta":{"code":429,"errorType":"quota_exceeded"}}"#,
        ),
        |err| matches!(err, Error::QuotaExceeded { .. }),
    )
    .await;
    assert_error_response(
        MockResponse::json(500, vec![], "server unavailable"),
        |err| matches!(err, Error::Server { .. }),
    )
    .await;
}

#[tokio::test]
async fn reports_malformed_success_response() {
    let server = MockServer::spawn(vec![MockResponse::json(200, vec![], "not json")]).await;
    let client = test_client(&server);

    let err = client
        .latest_checkins("token", CheckinsQuery::default())
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Decode(_)));
}

#[tokio::test]
async fn rejects_oversized_response_body() {
    let body = "x".repeat(MAX_RESPONSE_BODY_BYTES + 1);
    let server = MockServer::spawn(vec![MockResponse::json(500, vec![], &body)]).await;
    let client = test_client(&server);

    let err = client
        .latest_checkins("token", CheckinsQuery::default())
        .await
        .unwrap_err();

    assert!(matches!(err, Error::ResponseTooLarge { .. }));
}

async fn assert_error_response<F>(response: MockResponse, predicate: F)
where
    F: FnOnce(Error) -> bool,
{
    let server = MockServer::spawn(vec![response]).await;
    let client = test_client(&server);
    let err = client
        .latest_checkins("token", CheckinsQuery::default())
        .await
        .unwrap_err();
    assert!(predicate(err));
}

fn test_client(server: &MockServer) -> SwarmClient {
    SwarmClient::builder()
        .api_base_url(server.url("/v2/"))
        .oauth_authorize_url(server.url("/oauth2/authenticate"))
        .oauth_access_token_url(server.url("/oauth2/access_token"))
        .danger_accept_insecure_http_for_tests(true)
        .build()
        .unwrap()
}

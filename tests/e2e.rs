use beeline::{
    AuthorizationRequest, AuthorizedUser, CheckinsQuery, Error, LinkState, OAuthConfig,
    PollOptions, SwarmClient, parse_authorization_callback,
};
mod common;
use common::{MockResponse, MockServer};
use std::collections::HashMap;
use std::time::Duration;

const STATE_SIGNING_KEY: &[u8] = b"discord-bot-test-state-signing-key";

#[tokio::test]
async fn discord_link_oauth_exchange_then_fetches_latest_checkins() {
    let server = MockServer::spawn(vec![
        MockResponse::json(
            200,
            vec![],
            r#"{"access_token":"discord-bot-access-token","token_type":"bearer"}"#,
        ),
        MockResponse::json(
            200,
            vec![
                ("X-RateLimit-Limit", "500"),
                ("X-RateLimit-Remaining", "498"),
            ],
            r#"{
                "meta": {"code": 200, "requestId": "e2e-req"},
                "response": {
                    "checkins": {
                        "count": 2,
                        "items": [
                            {
                                "id": "checkin-a",
                                "createdAt": 1710000000,
                                "venue": {
                                    "id": "venue-a",
                                    "name": "Lunch Spot",
                                    "location": {"city": "Portland", "state": "OR"}
                                }
                            },
                            {
                                "id": "checkin-b",
                                "createdAt": 1710000300,
                                "shout": "coffee"
                            }
                        ]
                    }
                }
            }"#,
        ),
    ])
    .await;
    let client = test_client(&server);
    let oauth = OAuthConfig::new(
        "discord-bot-client",
        "discord-bot-secret",
        server.url("/callback").as_str(),
    )
    .unwrap();
    let link_state = LinkState::new(42, "nonce-for-db")
        .with_guild_id(7)
        .with_channel_id(9);
    let encoded_state = link_state.encode(STATE_SIGNING_KEY).unwrap();

    let authorize_url = client.authorization_url(
        &oauth,
        AuthorizationRequest::new().with_state(encoded_state.clone()),
    );
    let authorize_params: HashMap<String, String> =
        authorize_url.query_pairs().into_owned().collect();
    assert_eq!(
        authorize_params.get("client_id"),
        Some(&"discord-bot-client".to_string())
    );
    assert_eq!(authorize_params.get("state"), Some(&encoded_state));

    let callback = parse_authorization_callback(format!(
        "{}?code=oauth-code&state={encoded_state}",
        oauth.redirect_uri
    ))
    .unwrap();
    let decoded_state = LinkState::decode_with_max_age(
        callback.state.as_ref().unwrap(),
        STATE_SIGNING_KEY,
        Duration::from_secs(300),
    )
    .unwrap();
    assert_eq!(decoded_state, link_state);

    let token = client
        .exchange_code(&oauth, callback.code, None)
        .await
        .unwrap();
    assert_eq!(token.access_token, "discord-bot-access-token");

    let checkins = client
        .latest_checkins(
            &token.access_token,
            CheckinsQuery {
                limit: 2,
                offset: 0,
                version: Some("20260506".to_string()),
            },
        )
        .await
        .unwrap();

    assert_eq!(checkins.meta.request_id.as_deref(), Some("e2e-req"));
    assert_eq!(checkins.rate_limit.limit, Some(500));
    assert_eq!(checkins.rate_limit.remaining, Some(498));
    assert_eq!(checkins.checkins.count, Some(2));
    assert_eq!(checkins.checkins.items[0].id.as_deref(), Some("checkin-a"));
    assert_eq!(
        checkins.checkins.items[0]
            .venue
            .as_ref()
            .and_then(|venue| venue.name.as_deref()),
        Some("Lunch Spot")
    );

    let requests = server.requests().await;
    assert_eq!(requests.len(), 2);
    assert!(requests[0].contains("GET /oauth2/access_token?"));
    assert!(requests[0].contains("client_id=discord-bot-client"));
    assert!(requests[0].contains("client_secret=discord-bot-secret"));
    assert!(requests[0].contains("grant_type=authorization_code"));
    assert!(requests[0].contains("code=oauth-code"));

    let checkin_request = requests[1].to_ascii_lowercase();
    assert!(checkin_request.contains("get /v2/users/self/checkins?"));
    assert!(checkin_request.contains("authorization: bearer discord-bot-access-token"));
    assert!(checkin_request.contains("v=20260506"));
    assert!(checkin_request.contains("limit=2"));
    assert!(checkin_request.contains("offset=0"));
}

#[tokio::test]
async fn discord_multi_user_poll_returns_successes_and_link_failures_by_discord_id() {
    let server = MockServer::spawn(vec![
        MockResponse::json(
            200,
            vec![],
            r#"{
                "meta": {"code": 200},
                "response": {
                    "checkins": {
                        "count": 1,
                        "items": [{"id": "checkin-101", "createdAt": 1710000000}]
                    }
                }
            }"#,
        ),
        MockResponse::json(
            401,
            vec![],
            r#"{"meta":{"code":401,"errorType":"invalid_auth","errorDetail":"token revoked"}}"#,
        ),
        MockResponse::json(
            200,
            vec![],
            r#"{
                "meta": {"code": 200},
                "response": {
                    "checkins": {
                        "count": 1,
                        "items": [{"id": "checkin-303", "createdAt": 1710000900}]
                    }
                }
            }"#,
        ),
    ])
    .await;
    let client = test_client(&server);

    let results = client
        .latest_checkins_for_users(
            vec![
                AuthorizedUser {
                    external_user_id: 101_u64,
                    access_token: "token-101".to_string(),
                },
                AuthorizedUser {
                    external_user_id: 202_u64,
                    access_token: "revoked-token-202".to_string(),
                },
                AuthorizedUser {
                    external_user_id: 303_u64,
                    access_token: "token-303".to_string(),
                },
            ],
            PollOptions {
                max_concurrency: 1,
                query: CheckinsQuery {
                    limit: 1,
                    offset: 0,
                    version: Some("20260506".to_string()),
                },
            },
        )
        .await;

    let user_101 = results.get(&101).unwrap().as_ref().unwrap();
    assert_eq!(
        user_101.checkins.items[0].id.as_deref(),
        Some("checkin-101")
    );
    assert!(matches!(
        results.get(&202).unwrap(),
        Err(Error::InvalidAuth { .. })
    ));
    let user_303 = results.get(&303).unwrap().as_ref().unwrap();
    assert_eq!(
        user_303.checkins.items[0].id.as_deref(),
        Some("checkin-303")
    );

    let requests = server.requests().await;
    assert_eq!(requests.len(), 3);
    assert!(
        requests[0]
            .to_ascii_lowercase()
            .contains("authorization: bearer token-101")
    );
    assert!(
        requests[1]
            .to_ascii_lowercase()
            .contains("authorization: bearer revoked-token-202")
    );
    assert!(
        requests[2]
            .to_ascii_lowercase()
            .contains("authorization: bearer token-303")
    );
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

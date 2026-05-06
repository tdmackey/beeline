use crate::error::{Error, Result};
use crate::models::{ApiEnvelope, ApiMeta, CheckinsResult, RateLimit, UserCheckinsResponse};
use crate::oauth::{AccessToken, AuthorizationRequest, OAuthConfig};
use futures::{StreamExt, stream};
use reqwest::{StatusCode, header};
use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;
use std::time::Duration;
use url::Url;

pub const DEFAULT_API_VERSION: &str = "20260506";
const DEFAULT_API_BASE_URL: &str = "https://api.foursquare.com/v2/";
const DEFAULT_OAUTH_AUTHORIZE_URL: &str = "https://foursquare.com/oauth2/authenticate";
const DEFAULT_OAUTH_ACCESS_TOKEN_URL: &str = "https://foursquare.com/oauth2/access_token";
const DEFAULT_USER_AGENT: &str = "beeline/0.1";
const MAX_RESPONSE_BODY_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub struct SwarmClient {
    http: reqwest::Client,
    api_base_url: Url,
    oauth_authorize_url: Url,
    oauth_access_token_url: Url,
    api_version: String,
    user_agent: String,
}

impl SwarmClient {
    pub fn new() -> Self {
        Self::builder()
            .build()
            .expect("static default SwarmClient configuration is valid")
    }

    pub fn builder() -> SwarmClientBuilder {
        SwarmClientBuilder::default()
    }

    pub fn authorization_url(&self, config: &OAuthConfig, request: AuthorizationRequest) -> Url {
        let mut url = self.oauth_authorize_url.clone();
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("client_id", &config.client_id);
            pairs.append_pair("response_type", "code");
            pairs.append_pair("redirect_uri", request.redirect_uri(config).as_str());
            if let Some(state) = request.state {
                pairs.append_pair("state", &state);
            }
        }
        url
    }

    pub async fn exchange_code(
        &self,
        config: &OAuthConfig,
        code: impl AsRef<str>,
        redirect_uri_override: Option<&Url>,
    ) -> Result<AccessToken> {
        let redirect_uri = redirect_uri_override.unwrap_or(&config.redirect_uri);
        let response = self
            .http
            .get(self.oauth_access_token_url.clone())
            .header(header::ACCEPT, "application/json")
            .header(header::USER_AGENT, &self.user_agent)
            .query(&[
                ("client_id", config.client_id.as_str()),
                ("client_secret", config.client_secret.as_str()),
                ("grant_type", "authorization_code"),
                ("redirect_uri", redirect_uri.as_str()),
                ("code", code.as_ref()),
            ])
            .send()
            .await?;

        let status = response.status();
        let rate_limit = RateLimit::from_headers(response.headers());
        let body = response_body(response).await?;

        if !status.is_success() {
            return Err(map_api_error(status, rate_limit, &body));
        }

        Ok(serde_json::from_str(&body)?)
    }

    pub async fn latest_checkins(
        &self,
        access_token: impl AsRef<str>,
        query: CheckinsQuery,
    ) -> Result<CheckinsResult> {
        let url = self.api_base_url.join("users/self/checkins")?;
        let version = query
            .version
            .clone()
            .unwrap_or_else(|| self.api_version.clone());
        let limit = query.limit.to_string();
        let offset = query.offset.to_string();

        let response = self
            .http
            .get(url)
            .bearer_auth(access_token.as_ref())
            .header(header::ACCEPT, "application/json")
            .header(header::USER_AGENT, &self.user_agent)
            .query(&[
                ("v", version.as_str()),
                ("limit", limit.as_str()),
                ("offset", offset.as_str()),
            ])
            .send()
            .await?;

        let status = response.status();
        let rate_limit = RateLimit::from_headers(response.headers());
        let body = response_body(response).await?;

        if !status.is_success() {
            return Err(map_api_error(status, rate_limit, &body));
        }

        let envelope: ApiEnvelope<UserCheckinsResponse> = serde_json::from_str(&body)?;
        let response = envelope
            .response
            .ok_or_else(|| Error::MalformedResponse("missing response object".to_string()))?;

        Ok(CheckinsResult {
            meta: envelope.meta,
            rate_limit,
            checkins: response.checkins,
        })
    }

    pub async fn latest_checkins_for_users<Id, Users>(
        &self,
        users: Users,
        options: PollOptions,
    ) -> HashMap<Id, Result<CheckinsResult>>
    where
        Id: Clone + Eq + Hash + Send + 'static,
        Users: IntoIterator<Item = AuthorizedUser<Id>>,
    {
        let max_concurrency = options.max_concurrency.max(1);
        let query = options.query;
        let users: Vec<_> = users.into_iter().collect();

        stream::iter(users.into_iter().map(|user| {
            let client = self.clone();
            let query = query.clone();
            async move {
                let result = client.latest_checkins(&user.access_token, query).await;
                (user.external_user_id, result)
            }
        }))
        .buffer_unordered(max_concurrency)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect()
    }
}

impl Default for SwarmClient {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct SwarmClientBuilder {
    http: Option<reqwest::Client>,
    api_base_url: Url,
    oauth_authorize_url: Url,
    oauth_access_token_url: Url,
    api_version: String,
    user_agent: String,
    timeout: Duration,
    allow_insecure_endpoints: bool,
}

impl SwarmClientBuilder {
    pub fn http_client(mut self, http: reqwest::Client) -> Self {
        self.http = Some(http);
        self
    }

    pub fn api_base_url(mut self, url: Url) -> Self {
        self.api_base_url = url;
        self
    }

    pub fn oauth_authorize_url(mut self, url: Url) -> Self {
        self.oauth_authorize_url = url;
        self
    }

    pub fn oauth_access_token_url(mut self, url: Url) -> Self {
        self.oauth_access_token_url = url;
        self
    }

    pub fn api_version(mut self, api_version: impl Into<String>) -> Self {
        self.api_version = api_version.into();
        self
    }

    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn danger_accept_insecure_http_for_tests(mut self, allow: bool) -> Self {
        self.allow_insecure_endpoints = allow;
        self
    }

    pub fn build(self) -> Result<SwarmClient> {
        validate_endpoint(
            "api_base_url",
            &self.api_base_url,
            self.allow_insecure_endpoints,
        )?;
        validate_endpoint(
            "oauth_authorize_url",
            &self.oauth_authorize_url,
            self.allow_insecure_endpoints,
        )?;
        validate_endpoint(
            "oauth_access_token_url",
            &self.oauth_access_token_url,
            self.allow_insecure_endpoints,
        )?;

        let http = match self.http {
            Some(http) => http,
            None => reqwest::Client::builder()
                .timeout(self.timeout)
                .build()
                .map_err(Error::request)?,
        };

        Ok(SwarmClient {
            http,
            api_base_url: self.api_base_url,
            oauth_authorize_url: self.oauth_authorize_url,
            oauth_access_token_url: self.oauth_access_token_url,
            api_version: self.api_version,
            user_agent: self.user_agent,
        })
    }
}

impl Default for SwarmClientBuilder {
    fn default() -> Self {
        Self {
            http: None,
            api_base_url: Url::parse(DEFAULT_API_BASE_URL)
                .expect("static Foursquare API base URL is valid"),
            oauth_authorize_url: Url::parse(DEFAULT_OAUTH_AUTHORIZE_URL)
                .expect("static Foursquare OAuth authorize URL is valid"),
            oauth_access_token_url: Url::parse(DEFAULT_OAUTH_ACCESS_TOKEN_URL)
                .expect("static Foursquare OAuth token URL is valid"),
            api_version: DEFAULT_API_VERSION.to_string(),
            user_agent: DEFAULT_USER_AGENT.to_string(),
            timeout: Duration::from_secs(30),
            allow_insecure_endpoints: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckinsQuery {
    pub limit: u32,
    pub offset: u32,
    pub version: Option<String>,
}

impl Default for CheckinsQuery {
    fn default() -> Self {
        Self {
            limit: 50,
            offset: 0,
            version: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollOptions {
    pub query: CheckinsQuery,
    pub max_concurrency: usize,
}

impl Default for PollOptions {
    fn default() -> Self {
        Self {
            query: CheckinsQuery::default(),
            max_concurrency: 4,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizedUser<Id> {
    pub external_user_id: Id,
    pub access_token: String,
}

impl<Id: fmt::Debug> fmt::Debug for AuthorizedUser<Id> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthorizedUser")
            .field("external_user_id", &self.external_user_id)
            .field("access_token", &"<redacted>")
            .finish()
    }
}

fn validate_endpoint(name: &'static str, url: &Url, allow_insecure: bool) -> Result<()> {
    if allow_insecure || url.scheme() == "https" {
        return Ok(());
    }

    Err(Error::InsecureEndpoint {
        name,
        url: url.clone(),
    })
}

fn map_api_error(status: StatusCode, rate_limit: RateLimit, body: &str) -> Error {
    let meta = parse_error_meta(body);
    let error_type = meta
        .as_ref()
        .and_then(|meta| meta.error_type.as_deref())
        .map(str::to_string);
    let meta = meta.map(Box::new);
    let body = if body.is_empty() {
        None
    } else {
        Some(body.to_string())
    };

    match (status, error_type.as_deref()) {
        (StatusCode::BAD_REQUEST, _) | (_, Some("param_error")) => Error::BadRequest {
            meta,
            body,
            rate_limit,
        },
        (StatusCode::UNAUTHORIZED, _) | (_, Some("invalid_auth")) => Error::InvalidAuth {
            meta,
            body,
            rate_limit,
        },
        (_, Some("rate_limit_exceeded")) => Error::RateLimited {
            reset_at: rate_limit.reset_at,
            limit: rate_limit.limit,
            remaining: rate_limit.remaining,
            meta,
        },
        (StatusCode::FORBIDDEN, _) if rate_limit.reset_at.is_some() => Error::RateLimited {
            reset_at: rate_limit.reset_at,
            limit: rate_limit.limit,
            remaining: rate_limit.remaining,
            meta,
        },
        (StatusCode::FORBIDDEN, _) | (_, Some("not_authorized")) => Error::Forbidden {
            meta,
            body,
            rate_limit,
        },
        (StatusCode::TOO_MANY_REQUESTS, _) | (_, Some("quota_exceeded")) => Error::QuotaExceeded {
            meta,
            body,
            rate_limit,
        },
        (status, _) if status.is_server_error() => Error::Server {
            status,
            meta,
            body,
            rate_limit,
        },
        _ => Error::Api {
            status,
            meta,
            body,
            rate_limit,
        },
    }
}

async fn response_body(mut response: reqwest::Response) -> Result<String> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BODY_BYTES as u64)
    {
        return Err(Error::ResponseTooLarge {
            limit: MAX_RESPONSE_BODY_BYTES,
        });
    }

    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BODY_BYTES {
            return Err(Error::ResponseTooLarge {
                limit: MAX_RESPONSE_BODY_BYTES,
            });
        }
        bytes.extend_from_slice(&chunk);
    }

    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn parse_error_meta(body: &str) -> Option<ApiMeta> {
    if body.trim().is_empty() {
        return None;
    }

    serde_json::from_str::<ApiEnvelope<serde_json::Value>>(body)
        .map(|envelope| envelope.meta)
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AccessToken, AuthorizationRequest, Error, LinkState, OAuthConfig,
        parse_authorization_callback,
    };
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    const STATE_SIGNING_KEY: &[u8] = b"test-state-signing-key";

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
    }

    #[test]
    fn debug_redacts_secret_bearing_types() {
        let oauth = OAuthConfig::new(
            "client",
            "client-secret",
            "https://bot.example.com/callback",
        )
        .unwrap();
        let token = AccessToken {
            access_token: "access-token".to_string(),
            token_type: Some("bearer".to_string()),
            scope: None,
            extra: BTreeMap::new(),
        };
        let user = AuthorizedUser {
            external_user_id: 42_u64,
            access_token: "user-token".to_string(),
        };

        let rendered = format!("{oauth:?} {token:?} {user:?}");

        assert!(!rendered.contains("client-secret"));
        assert!(!rendered.contains("access-token"));
        assert!(!rendered.contains("user-token"));
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
        let config =
            OAuthConfig::new("client", "secret", "https://bot.example.com/callback").unwrap();

        let token = client
            .exchange_code(&config, "code-123", None)
            .await
            .unwrap();

        assert_eq!(token.access_token, "access-token");
        let requests = server.requests().await;
        assert_eq!(requests.len(), 1);
        assert!(requests[0].contains("GET /oauth2/access_token?"));
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

    struct MockServer {
        base_url: Url,
        requests: Arc<Mutex<Vec<String>>>,
    }

    impl MockServer {
        async fn spawn(responses: Vec<MockResponse>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let requests_for_task = requests.clone();

            tokio::spawn(async move {
                for response in responses {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut buf = vec![0_u8; 8192];
                    let mut raw = Vec::new();
                    loop {
                        let n = socket.read(&mut buf).await.unwrap();
                        if n == 0 {
                            break;
                        }
                        raw.extend_from_slice(&buf[..n]);
                        if raw.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }

                    let request = String::from_utf8_lossy(&raw).to_string();
                    requests_for_task.lock().await.push(request);
                    socket
                        .write_all(response.to_http().as_bytes())
                        .await
                        .unwrap();
                    socket.shutdown().await.unwrap();
                }
            });

            Self {
                base_url: Url::parse(&format!("http://{addr}/")).unwrap(),
                requests,
            }
        }

        fn url(&self, path: &str) -> Url {
            self.base_url.join(path.trim_start_matches('/')).unwrap()
        }

        async fn requests(&self) -> Vec<String> {
            self.requests.lock().await.clone()
        }
    }

    struct MockResponse {
        status: u16,
        headers: Vec<(&'static str, &'static str)>,
        body: String,
    }

    impl MockResponse {
        fn json(status: u16, headers: Vec<(&'static str, &'static str)>, body: &str) -> Self {
            Self {
                status,
                headers,
                body: body.to_string(),
            }
        }

        fn to_http(&self) -> String {
            let reason = match self.status {
                200 => "OK",
                400 => "Bad Request",
                401 => "Unauthorized",
                403 => "Forbidden",
                429 => "Too Many Requests",
                500 => "Internal Server Error",
                _ => "OK",
            };

            let mut response = format!(
                "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                self.status,
                reason,
                self.body.len()
            );
            for (name, value) in &self.headers {
                response.push_str(name);
                response.push_str(": ");
                response.push_str(value);
                response.push_str("\r\n");
            }
            response.push_str("\r\n");
            response.push_str(&self.body);
            response
        }
    }
}

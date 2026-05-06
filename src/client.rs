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
const DEFAULT_USER_AGENT: &str = concat!("beeline/", env!("CARGO_PKG_VERSION"));
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
        let api_base_url = normalize_base_url(self.api_base_url);

        validate_endpoint("api_base_url", &api_base_url, self.allow_insecure_endpoints)?;
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
            api_base_url,
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

fn normalize_base_url(mut url: Url) -> Url {
    if !url.path().ends_with('/') {
        let mut path = url.path().to_string();
        path.push('/');
        url.set_path(&path);
    }

    url
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

use crate::models::{ApiMeta, RateLimit};
use reqwest::StatusCode;
use thiserror::Error as ThisError;
use url::Url;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(ThisError, Debug)]
pub enum Error {
    #[error("failed to build URL: {0}")]
    Url(#[from] url::ParseError),

    #[error("HTTP request failed: {source}")]
    Request {
        #[source]
        source: reqwest::Error,
    },

    #[error("failed to decode JSON response: {0}")]
    Decode(#[from] serde_json::Error),

    #[error("Foursquare callback returned error {error}: {description:?}")]
    CallbackDenied {
        error: String,
        description: Option<String>,
    },

    #[error("authorization callback did not include a code")]
    MissingCallbackCode,

    #[error("invalid OAuth state: {0}")]
    InvalidState(String),

    #[error("OAuth state has expired")]
    ExpiredState {
        issued_at: i64,
        max_age_seconds: u64,
    },

    #[error("bad request to Foursquare API")]
    BadRequest {
        meta: Option<Box<ApiMeta>>,
        body: Option<String>,
        rate_limit: RateLimit,
    },

    #[error("OAuth token is missing, invalid, or expired")]
    InvalidAuth {
        meta: Option<Box<ApiMeta>>,
        body: Option<String>,
        rate_limit: RateLimit,
    },

    #[error("Foursquare denied access for this token")]
    Forbidden {
        meta: Option<Box<ApiMeta>>,
        body: Option<String>,
        rate_limit: RateLimit,
    },

    #[error("Foursquare rate limit exceeded")]
    RateLimited {
        reset_at: Option<i64>,
        limit: Option<u64>,
        remaining: Option<u64>,
        meta: Option<Box<ApiMeta>>,
    },

    #[error("Foursquare daily quota exceeded")]
    QuotaExceeded {
        meta: Option<Box<ApiMeta>>,
        body: Option<String>,
        rate_limit: RateLimit,
    },

    #[error("Foursquare server error: {status}")]
    Server {
        status: StatusCode,
        meta: Option<Box<ApiMeta>>,
        body: Option<String>,
        rate_limit: RateLimit,
    },

    #[error("Foursquare API error: {status}")]
    Api {
        status: StatusCode,
        meta: Option<Box<ApiMeta>>,
        body: Option<String>,
        rate_limit: RateLimit,
    },

    #[error("malformed Foursquare response: {0}")]
    MalformedResponse(String),

    #[error("Foursquare response body exceeded {limit} bytes")]
    ResponseTooLarge { limit: usize },

    #[error("insecure {name} endpoint rejected: {url}")]
    InsecureEndpoint { name: &'static str, url: Url },
}

impl Error {
    pub(crate) fn request(source: reqwest::Error) -> Self {
        Self::Request {
            source: source.without_url(),
        }
    }

    pub fn is_invalid_auth(&self) -> bool {
        matches!(self, Error::InvalidAuth { .. })
    }

    pub fn is_rate_limited(&self) -> bool {
        matches!(
            self,
            Error::RateLimited { .. } | Error::QuotaExceeded { .. }
        )
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self, Error::RateLimited { .. } | Error::Server { .. })
    }
}

impl From<reqwest::Error> for Error {
    fn from(source: reqwest::Error) -> Self {
        Self::request(source)
    }
}

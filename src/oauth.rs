use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use url::Url;

#[derive(Clone, PartialEq, Eq)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: Url,
}

impl OAuthConfig {
    pub fn new(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        redirect_uri: impl AsRef<str>,
    ) -> Result<Self> {
        Ok(Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            redirect_uri: Url::parse(redirect_uri.as_ref())?,
        })
    }
}

impl fmt::Debug for OAuthConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthConfig")
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .field("redirect_uri", &self.redirect_uri)
            .finish()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthorizationRequest {
    pub state: Option<String>,
    pub redirect_uri_override: Option<Url>,
}

impl AuthorizationRequest {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_state(mut self, state: impl Into<String>) -> Self {
        self.state = Some(state.into());
        self
    }

    pub fn with_redirect_uri_override(mut self, redirect_uri: Url) -> Self {
        self.redirect_uri_override = Some(redirect_uri);
        self
    }

    pub fn redirect_uri<'a>(&'a self, config: &'a OAuthConfig) -> &'a Url {
        self.redirect_uri_override
            .as_ref()
            .unwrap_or(&config.redirect_uri)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AuthorizationCallback {
    pub code: String,
    pub state: Option<String>,
}

impl fmt::Debug for AuthorizationCallback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthorizationCallback")
            .field("code", &"<redacted>")
            .field("state", &self.state.as_ref().map(|_| "<present>"))
            .finish()
    }
}

pub fn parse_authorization_callback(
    callback_url: impl AsRef<str>,
) -> Result<AuthorizationCallback> {
    let url = Url::parse(callback_url.as_ref())?;
    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut error_description = None;

    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => code = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            "error" => error = Some(value.into_owned()),
            "error_description" => error_description = Some(value.into_owned()),
            _ => {}
        }
    }

    if let Some(error) = error {
        return Err(Error::CallbackDenied {
            error,
            description: error_description,
        });
    }

    let code = code.ok_or(Error::MissingCallbackCode)?;
    Ok(AuthorizationCallback { code, state })
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
pub struct AccessToken {
    pub access_token: String,
    #[serde(default)]
    pub token_type: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessToken")
            .field("access_token", &"<redacted>")
            .field("token_type", &self.token_type)
            .field("scope", &self.scope)
            .field("extra", &self.extra)
            .finish()
    }
}

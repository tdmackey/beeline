use crate::{Error, Result};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::time::Duration;

type HmacSha256 = Hmac<Sha256>;
const SIGNED_STATE_VERSION: &str = "v1";
const MAX_FUTURE_STATE_SKEW_SECONDS: i64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LinkState {
    pub discord_user_id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<u64>,
    pub nonce: String,
    pub issued_at: i64,
}

impl LinkState {
    pub fn new(discord_user_id: u64, nonce: impl Into<String>) -> Self {
        Self {
            discord_user_id,
            guild_id: None,
            channel_id: None,
            nonce: nonce.into(),
            issued_at: Utc::now().timestamp(),
        }
    }

    pub fn with_guild_id(mut self, guild_id: u64) -> Self {
        self.guild_id = Some(guild_id);
        self
    }

    pub fn with_channel_id(mut self, channel_id: u64) -> Self {
        self.channel_id = Some(channel_id);
        self
    }

    pub fn with_issued_at(mut self, issued_at: i64) -> Self {
        self.issued_at = issued_at;
        self
    }

    pub fn encode(&self, signing_key: impl AsRef<[u8]>) -> Result<String> {
        let bytes = serde_json::to_vec(self)?;
        let payload = URL_SAFE_NO_PAD.encode(bytes);
        let signature = sign_payload(&payload, signing_key.as_ref())?;
        Ok(format!(
            "{SIGNED_STATE_VERSION}.{payload}.{}",
            URL_SAFE_NO_PAD.encode(signature)
        ))
    }

    pub fn decode(encoded: impl AsRef<str>, signing_key: impl AsRef<[u8]>) -> Result<Self> {
        let encoded = encoded.as_ref();
        let mut parts = encoded.split('.');
        let version = parts
            .next()
            .ok_or_else(|| Error::InvalidState("missing state version".to_string()))?;
        let payload = parts
            .next()
            .ok_or_else(|| Error::InvalidState("missing state payload".to_string()))?;
        let signature = parts
            .next()
            .ok_or_else(|| Error::InvalidState("missing state signature".to_string()))?;

        if parts.next().is_some() {
            return Err(Error::InvalidState("too many state sections".to_string()));
        }

        if version != SIGNED_STATE_VERSION {
            return Err(Error::InvalidState("unsupported state version".to_string()));
        }

        let signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|err| Error::InvalidState(err.to_string()))?;
        verify_payload(payload, signing_key.as_ref(), &signature)?;
        Self::decode_payload(payload)
    }

    pub fn decode_with_max_age(
        encoded: impl AsRef<str>,
        signing_key: impl AsRef<[u8]>,
        max_age: Duration,
    ) -> Result<Self> {
        let state = Self::decode(encoded, signing_key)?;
        state.validate_max_age(max_age)
    }

    fn decode_payload(payload: &str) -> Result<Self> {
        let bytes = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|err| Error::InvalidState(err.to_string()))?;
        serde_json::from_slice(&bytes).map_err(Error::Decode)
    }

    fn validate_max_age(self, max_age: Duration) -> Result<Self> {
        let now = Utc::now().timestamp();
        let max_age_seconds = max_age.as_secs();
        let future_skew_seconds = self.issued_at.saturating_sub(now);

        if future_skew_seconds > MAX_FUTURE_STATE_SKEW_SECONDS {
            return Err(Error::InvalidState(
                "state issued_at is too far in the future".to_string(),
            ));
        }

        if now.saturating_sub(self.issued_at) > max_age_seconds as i64 {
            return Err(Error::ExpiredState {
                issued_at: self.issued_at,
                max_age_seconds,
            });
        }

        Ok(self)
    }
}

fn sign_payload(payload: &str, signing_key: &[u8]) -> Result<Vec<u8>> {
    let mut mac = state_mac(signing_key)?;
    mac.update(SIGNED_STATE_VERSION.as_bytes());
    mac.update(b".");
    mac.update(payload.as_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}

fn verify_payload(payload: &str, signing_key: &[u8], signature: &[u8]) -> Result<()> {
    let mut mac = state_mac(signing_key)?;
    mac.update(SIGNED_STATE_VERSION.as_bytes());
    mac.update(b".");
    mac.update(payload.as_bytes());
    mac.verify_slice(signature)
        .map_err(|_| Error::InvalidState("invalid state signature".to_string()))
}

fn state_mac(signing_key: &[u8]) -> Result<HmacSha256> {
    if signing_key.is_empty() {
        return Err(Error::InvalidState(
            "state signing key must not be empty".to_string(),
        ));
    }

    HmacSha256::new_from_slice(signing_key)
        .map_err(|_| Error::InvalidState("invalid state signing key".to_string()))
}

//! Async Rust client helpers for Foursquare/Swarm user OAuth and checkin polling.
//!
//! `beeline` is intentionally storage- and framework-neutral. A caller such as a
//! Discord bot can use signed [`LinkState`] values to carry Discord metadata
//! through OAuth, store tokens in its own database, then poll checkins with
//! [`SwarmClient`].

mod client;
mod error;
mod link_state;
mod models;
mod oauth;

pub use client::{
    AuthorizedUser, CheckinsQuery, DEFAULT_API_VERSION, PollOptions, SwarmClient,
    SwarmClientBuilder,
};
pub use error::{Error, Result};
pub use link_state::LinkState;
pub use models::*;
pub use oauth::{
    AccessToken, AuthorizationCallback, AuthorizationRequest, OAuthConfig,
    parse_authorization_callback,
};

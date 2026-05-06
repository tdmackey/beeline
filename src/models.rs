use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RateLimit {
    pub limit: Option<u64>,
    pub remaining: Option<u64>,
    pub reset_at: Option<i64>,
}

impl RateLimit {
    pub fn from_headers(headers: &HeaderMap) -> Self {
        Self {
            limit: parse_u64_header(headers, "x-ratelimit-limit"),
            remaining: parse_u64_header(headers, "x-ratelimit-remaining"),
            reset_at: parse_i64_header(headers, "x-ratelimit-reset"),
        }
    }
}

fn parse_u64_header(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
}

fn parse_i64_header(headers: &HeaderMap, name: &str) -> Option<i64> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok())
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApiMeta {
    pub code: Option<u16>,
    pub request_id: Option<String>,
    pub error_type: Option<String>,
    pub error_detail: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ApiEnvelope<T> {
    #[serde(default)]
    pub meta: ApiMeta,
    pub response: Option<T>,
    #[serde(default)]
    pub notifications: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct UserCheckinsResponse {
    #[serde(default)]
    pub checkins: CountItems<Checkin>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CheckinsResult {
    pub meta: ApiMeta,
    pub rate_limit: RateLimit,
    pub checkins: CountItems<Checkin>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(bound(deserialize = "T: Deserialize<'de>", serialize = "T: Serialize"))]
pub struct CountItems<T> {
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub items: Vec<T>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl<T> Default for CountItems<T> {
    fn default() -> Self {
        Self {
            count: None,
            items: Vec::new(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Checkin {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default, rename = "type")]
    pub checkin_type: Option<String>,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub time_zone: Option<String>,
    #[serde(default)]
    pub time_zone_offset: Option<i32>,
    #[serde(default)]
    pub user: Option<User>,
    #[serde(default)]
    pub venue: Option<Venue>,
    #[serde(default)]
    pub shout: Option<String>,
    #[serde(default)]
    pub photos: Option<CountItems<Photo>>,
    #[serde(default)]
    pub comments: Option<CountItems<Comment>>,
    #[serde(default)]
    pub likes: Option<CountItems<Like>>,
    #[serde(default)]
    pub source: Option<Source>,
    #[serde(default)]
    pub event: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct User {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub gender: Option<String>,
    #[serde(default)]
    pub photo: Option<Photo>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Venue {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub location: Option<Location>,
    #[serde(default)]
    pub categories: Vec<Category>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub rating: Option<f64>,
    #[serde(default)]
    pub stats: Option<Stats>,
    #[serde(default)]
    pub photos: Option<CountItems<Photo>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Location {
    #[serde(default)]
    pub address: Option<String>,
    #[serde(default)]
    pub cross_street: Option<String>,
    #[serde(default)]
    pub lat: Option<f64>,
    #[serde(default)]
    pub lng: Option<f64>,
    #[serde(default)]
    pub postal_code: Option<String>,
    #[serde(default)]
    pub cc: Option<String>,
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub formatted_address: Vec<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub plural_name: Option<String>,
    #[serde(default)]
    pub short_name: Option<String>,
    #[serde(default)]
    pub icon: Option<Icon>,
    #[serde(default)]
    pub primary: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Icon {
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub suffix: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Photo {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub prefix: Option<String>,
    #[serde(default)]
    pub suffix: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub visibility: Option<String>,
    #[serde(default)]
    pub user: Option<Box<User>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub created_at: Option<i64>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub user: Option<User>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Like {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub first_name: Option<String>,
    #[serde(default)]
    pub last_name: Option<String>,
    #[serde(default)]
    pub photo: Option<Photo>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
pub struct Source {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    #[serde(default)]
    pub checkins_count: Option<u64>,
    #[serde(default)]
    pub users_count: Option<u64>,
    #[serde(default)]
    pub tip_count: Option<u64>,
    #[serde(default)]
    pub visits_count: Option<u64>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

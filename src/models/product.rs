use mongodb::bson::{DateTime, oid::ObjectId};
use serde::{Deserialize, Serialize};
use url::Url;

pub const COLLECTION: &str = "products";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Product {
    #[serde(rename = "_id")]
    pub id: ObjectId,
    pub name: String,
    pub notes: Option<String>,
    #[serde(default)]
    pub is_favorite: bool,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    pub stores: Vec<Store>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Store {
    pub id: ObjectId,
    pub url: String,
    pub selector: String,
    pub currency: String,
    pub fetch_mode: FetchMode,
    pub status: StoreStatus,
    pub last_error: Option<LastError>,
    #[serde(default)]
    pub last_success_at: Option<DateTime>,
    pub created_at: DateTime,
    pub price_history: Vec<PricePoint>,
}

impl Store {
    pub fn domain(&self) -> String {
        Url::parse(&self.url)
            .ok()
            .and_then(|u| {
                u.host_str()
                    .map(|h| h.trim_start_matches("www.").to_lowercase())
            })
            .unwrap_or_else(|| self.url.clone())
    }

    pub fn last_price(&self) -> Option<f64> {
        self.price_history.last().map(|p| p.price)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchMode {
    Http,
    Browser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreStatus {
    Ok,
    Failing,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastError {
    pub date: DateTime,
    pub reason: FailureReason,
    pub since: DateTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    ElementNotFound,
    PageUnavailable,
    InvalidText,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricePoint {
    pub date: DateTime,
    pub price: f64,
}

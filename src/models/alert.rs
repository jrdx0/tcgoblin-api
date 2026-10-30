use std::time::Duration;

use mongodb::bson::{DateTime, oid::ObjectId};
use serde::{Deserialize, Serialize};

use super::product::FailureReason;

pub const COLLECTION: &str = "alerts";
pub const TTL_INDEX: &str = "alerts_ttl";
// 90 days
pub const TTL: Duration = Duration::from_secs(90 * 24 * 60 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Alert {
    #[serde(rename = "_id")]
    pub id: ObjectId,
    pub created_at: DateTime,
    pub kind: AlertKind,
    pub product_id: ObjectId,
    pub product_name: String,
    pub store_id: ObjectId,
    pub store_url: String,
    pub currency: String,
    pub previous_price: Option<f64>,
    pub price: Option<f64>,
    pub below_average: bool,
    pub failure: Option<AlertFailure>,
    pub delivery: Delivery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    PriceDrop,
    PriceRise,
    Failure,
    Recovery,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertFailure {
    pub reason: FailureReason,
    pub message: Option<String>,
    pub last_success_at: Option<DateTime>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub status: DeliveryStatus,
    pub attempts: u32,
    pub last_attempt_at: Option<DateTime>,
    pub error: Option<String>,
    pub next_attempt_at: Option<DateTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    Pending,
    Sending,
    Sent,
    Failed,
}

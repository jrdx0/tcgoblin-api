use std::time::Duration;

use mongodb::bson::{DateTime, oid::ObjectId};
use serde::{Deserialize, Serialize};

use super::product::{FailureReason, FetchMode};

pub const COLLECTION: &str = "checks";
pub const TTL_INDEX: &str = "checks_ttl";
// 30 days. Keep above 1 day or the scheduler re-enqueues expired slots.
pub const TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    #[serde(rename = "_id")]
    pub id: ObjectId,
    pub product_id: ObjectId,
    pub store_id: ObjectId,
    pub domain: String,
    pub scheduled_for: DateTime,
    pub attempt: u32,
    pub status: CheckStatus,
    pub locked_until: Option<DateTime>,
    pub created_at: DateTime,
    pub started_at: Option<DateTime>,
    pub finished_at: Option<DateTime>,
    pub result: Option<CheckResult>,
    pub fetch_mode: Option<FetchMode>,
    pub text: Option<String>,
    pub price: Option<f64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pending,
    Running,
    Done,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckResult {
    Initial,
    Unchanged,
    PriceChanged,
    ElementNotFound,
    PageUnavailable,
    InvalidText,
}

impl From<FailureReason> for CheckResult {
    fn from(reason: FailureReason) -> Self {
        match reason {
            FailureReason::ElementNotFound => CheckResult::ElementNotFound,
            FailureReason::PageUnavailable => CheckResult::PageUnavailable,
            FailureReason::InvalidText => CheckResult::InvalidText,
        }
    }
}

impl Check {
    pub fn pending(
        product_id: ObjectId,
        store_id: ObjectId,
        domain: String,
        scheduled_for: DateTime,
        attempt: u32,
    ) -> Self {
        Self {
            id: ObjectId::new(),
            product_id,
            store_id,
            domain,
            scheduled_for,
            attempt,
            status: CheckStatus::Pending,
            locked_until: None,
            created_at: DateTime::now(),
            started_at: None,
            finished_at: None,
            result: None,
            fetch_mode: None,
            text: None,
            price: None,
            error: None,
        }
    }
}

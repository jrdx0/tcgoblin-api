use mongodb::bson::DateTime;
use serde::{Deserialize, Serialize};

pub const COLLECTION: &str = "settings";
pub const ID: &str = "app";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    #[serde(rename = "_id")]
    pub id: String,
    pub discord: DiscordSettings,
    pub daily_check: DailyCheck,
    pub limits: Limits,
    pub updated_at: DateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscordSettings {
    pub webhook_url: Option<String>,
    pub notify_recovery: bool,
    pub last_test: Option<WebhookTest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookTest {
    pub sent_at: DateTime,
    pub delivered: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyCheck {
    pub time: String,
    pub timezone: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    pub page_timeout_seconds: u32,
    pub store_delay_seconds: u32,
    pub retry_after_minutes: u32,
}

impl Settings {
    pub fn defaults(now: DateTime) -> Self {
        Self {
            id: ID.to_string(),
            discord: DiscordSettings {
                webhook_url: None,
                notify_recovery: true,
                last_test: None,
            },
            daily_check: DailyCheck {
                time: "02:00".to_string(),
                timezone: "America/Mexico_City".to_string(),
            },
            limits: Limits {
                page_timeout_seconds: 30,
                store_delay_seconds: 5,
                retry_after_minutes: 30,
            },
            updated_at: now,
        }
    }
}

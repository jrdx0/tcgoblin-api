use axum::{Json, extract::State};
use chrono_tz::Tz;
use mongodb::{
    bson::{DateTime, Document, doc, to_bson},
    options::ReturnDocument,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use url::Url;

use crate::{
    AppState,
    error::AppError,
    models::settings::{self, DailyCheck, Limits, Settings, WebhookTest},
    routes::products::to_rfc3339,
};

const MASKED_WEBHOOK: &str = "https://discord.com/api/webhooks/••••••••";
const TEST_EMBED_COLOR: u32 = 0x5865F2;

pub async fn get(State(state): State<AppState>) -> Result<Json<SettingsResponse>, AppError> {
    let settings = state
        .db
        .collection::<Settings>(settings::COLLECTION)
        .find_one(doc! { "_id": settings::ID })
        .await?
        .ok_or_else(|| AppError::Internal("settings document is missing".to_string()))?;

    Ok(Json(settings.into()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSettings {
    discord: UpdateDiscord,
    daily_check: DailyCheck,
    limits: Limits,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDiscord {
    notify_recovery: bool,
}

pub async fn update(
    State(state): State<AppState>,
    Json(body): Json<UpdateSettings>,
) -> Result<Json<SettingsResponse>, AppError> {
    let daily_check = validate_daily_check(body.daily_check)?;
    let limits = validate_limits(body.limits)?;

    let settings = save(
        &state,
        doc! {
            "discord.notifyRecovery": body.discord.notify_recovery,
            "dailyCheck": to_bson(&daily_check)?,
            "limits": to_bson(&limits)?,
        },
    )
    .await?;

    Ok(Json(settings.into()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWebhook {
    webhook_url: Option<String>,
}

pub async fn update_webhook(
    State(state): State<AppState>,
    Json(body): Json<UpdateWebhook>,
) -> Result<Json<SettingsResponse>, AppError> {
    let encrypted = match body.webhook_url {
        Some(url) => Some(state.cipher.encrypt(&validate_webhook_url(&url)?)),
        None => None,
    };

    let settings = save(
        &state,
        doc! {
            "discord.webhookUrl": encrypted,
            "discord.lastTest": null,
        },
    )
    .await?;

    Ok(Json(settings.into()))
}

pub async fn test_webhook(
    State(state): State<AppState>,
) -> Result<Json<WebhookTestResponse>, AppError> {
    let settings = state
        .db
        .collection::<Settings>(settings::COLLECTION)
        .find_one(doc! { "_id": settings::ID })
        .await?
        .ok_or_else(|| AppError::Internal("settings document is missing".to_string()))?;

    let encrypted = settings
        .discord
        .webhook_url
        .ok_or_else(|| AppError::Validation("webhook is not configured".to_string()))?;
    let webhook_url = state
        .cipher
        .decrypt(&encrypted)
        .map_err(AppError::Internal)?;

    let sent_at = DateTime::now();
    let message = json!({
        "embeds": [{
            "title": "Mensaje de prueba",
            "description": "TCGoblin puede enviar alertas a este canal.",
            "color": TEST_EMBED_COLOR,
            "timestamp": to_rfc3339(sent_at),
        }]
    });
    let error = state.discord.send(&webhook_url, &message).await.err();

    let test = WebhookTest {
        sent_at,
        delivered: error.is_none(),
        error,
    };

    state
        .db
        .collection::<Settings>(settings::COLLECTION)
        .update_one(
            doc! { "_id": settings::ID },
            doc! { "$set": { "discord.lastTest": to_bson(&test)? } },
        )
        .await?;

    Ok(Json(test.into()))
}

async fn save(state: &AppState, mut set: Document) -> Result<Settings, AppError> {
    set.insert("updatedAt", DateTime::now());

    state
        .db
        .collection::<Settings>(settings::COLLECTION)
        .find_one_and_update(doc! { "_id": settings::ID }, doc! { "$set": set })
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::Internal("settings document is missing".to_string()))
}

fn validate_daily_check(daily_check: DailyCheck) -> Result<DailyCheck, AppError> {
    let time = daily_check.time.trim();
    let valid_time = time
        .split_once(':')
        .filter(|(h, m)| h.len() == 2 && m.len() == 2)
        .and_then(|(h, m)| Some((h.parse::<u8>().ok()?, m.parse::<u8>().ok()?)))
        .is_some_and(|(h, m)| h < 24 && m < 60);
    if !valid_time {
        return Err(AppError::Validation(
            "dailyCheck.time must be HH:mm in 24 h format".to_string(),
        ));
    }

    let timezone = daily_check.timezone.trim();
    if timezone.parse::<Tz>().is_err() {
        return Err(AppError::Validation(
            "dailyCheck.timezone must be an IANA timezone name".to_string(),
        ));
    }

    Ok(DailyCheck {
        time: time.to_string(),
        timezone: timezone.to_string(),
    })
}

fn validate_limits(limits: Limits) -> Result<Limits, AppError> {
    if limits.page_timeout_seconds < 5 {
        return Err(AppError::Validation(
            "limits.pageTimeoutSeconds must be at least 5".to_string(),
        ));
    }
    if limits.retry_after_minutes < 1 {
        return Err(AppError::Validation(
            "limits.retryAfterMinutes must be at least 1".to_string(),
        ));
    }
    Ok(limits)
}

fn validate_webhook_url(url: &str) -> Result<String, AppError> {
    Url::parse(url.trim())
        .ok()
        .filter(|u| {
            u.scheme() == "https"
                && matches!(
                    u.host_str(),
                    Some(
                        "discord.com" | "discordapp.com" | "ptb.discord.com" | "canary.discord.com"
                    )
                )
                && u.path().starts_with("/api/webhooks/")
        })
        .map(|u| u.to_string())
        .ok_or_else(|| {
            AppError::Validation(
                "webhookUrl must be a Discord webhook URL (https://discord.com/api/webhooks/…)"
                    .to_string(),
            )
        })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsResponse {
    discord: DiscordResponse,
    daily_check: DailyCheck,
    limits: Limits,
    updated_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscordResponse {
    webhook_configured: bool,
    webhook_url: Option<&'static str>,
    notify_recovery: bool,
    last_test: Option<WebhookTestResponse>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebhookTestResponse {
    sent_at: String,
    delivered: bool,
    error: Option<String>,
}

impl From<Settings> for SettingsResponse {
    fn from(s: Settings) -> Self {
        let configured = s.discord.webhook_url.is_some();
        Self {
            discord: DiscordResponse {
                webhook_configured: configured,
                webhook_url: configured.then_some(MASKED_WEBHOOK),
                notify_recovery: s.discord.notify_recovery,
                last_test: s.discord.last_test.map(Into::into),
            },
            daily_check: s.daily_check,
            limits: s.limits,
            updated_at: to_rfc3339(s.updated_at),
        }
    }
}

impl From<WebhookTest> for WebhookTestResponse {
    fn from(t: WebhookTest) -> Self {
        Self {
            sent_at: to_rfc3339(t.sent_at),
            delivered: t.delivered,
            error: t.error,
        }
    }
}

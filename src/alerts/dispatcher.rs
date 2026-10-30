use std::{sync::Arc, time::Duration};

use mongodb::{
    Collection, Database,
    bson::{DateTime, doc},
    options::ReturnDocument,
};
use tokio::sync::Notify;

use super::messages;
use crate::{
    crypto::Cipher,
    discord::DiscordClient,
    models::{
        alert::{self, Alert},
        settings::{self, Settings},
    },
};

const POLL_INTERVAL: Duration = Duration::from_secs(10);
const LEASE: Duration = Duration::from_secs(2 * 60);
const RETRY_BACKOFF: Duration = Duration::from_secs(60);
const MAX_ATTEMPTS: u32 = 5;
const MAX_EMBEDS: usize = 10;
const MESSAGE_GAP: Duration = Duration::from_secs(1);

pub async fn run(db: Database, cipher: Arc<Cipher>, discord: DiscordClient, signal: Arc<Notify>) {
    loop {
        if let Err(err) = dispatch(&db, &cipher, &discord).await {
            eprintln!("alert dispatcher: {err}");
        }

        tokio::select! {
            _ = signal.notified() => {}
            _ = tokio::time::sleep(POLL_INTERVAL) => {}
        }
    }
}

async fn dispatch(db: &Database, cipher: &Cipher, discord: &DiscordClient) -> Result<(), String> {
    let settings = db
        .collection::<Settings>(settings::COLLECTION)
        .find_one(doc! { "_id": settings::ID })
        .await
        .map_err(|e| format!("failed to read settings: {e}"))?
        .ok_or("settings document is missing")?;

    let Some(encrypted) = settings.discord.webhook_url else {
        return Ok(());
    };
    let webhook_url = cipher.decrypt(&encrypted)?;

    let alerts = db.collection::<Alert>(alert::COLLECTION);
    let mut first_message = true;

    loop {
        let mut batch = Vec::new();
        while batch.len() < MAX_EMBEDS {
            match claim(&alerts).await? {
                Some(alert) => batch.push(alert),
                None => break,
            }
        }
        if batch.is_empty() {
            return Ok(());
        }

        if !first_message {
            tokio::time::sleep(MESSAGE_GAP).await;
        }
        first_message = false;

        let result = discord.send(&webhook_url, &messages::build(&batch)).await;
        record(&alerts, &batch, result).await?;
    }
}

async fn claim(alerts: &Collection<Alert>) -> Result<Option<Alert>, String> {
    let now = DateTime::now();

    alerts
        .find_one_and_update(
            doc! {
                "delivery.status": { "$in": ["pending", "sending"] },
                "$or": [
                    { "delivery.nextAttemptAt": null },
                    { "delivery.nextAttemptAt": { "$lte": now } },
                ],
            },
            doc! {
                "$set": {
                    "delivery.status": "sending",
                    "delivery.nextAttemptAt": after(now, LEASE),
                },
            },
        )
        .sort(doc! { "createdAt": 1, "_id": 1 })
        .return_document(ReturnDocument::After)
        .await
        .map_err(|e| format!("failed to claim alert: {e}"))
}

async fn record(
    alerts: &Collection<Alert>,
    batch: &[Alert],
    result: Result<(), String>,
) -> Result<(), String> {
    let now = DateTime::now();

    for alert in batch {
        let attempts = alert.delivery.attempts + 1;
        let update = match &result {
            Ok(()) => doc! {
                "delivery.status": "sent",
                "delivery.attempts": i64::from(attempts),
                "delivery.lastAttemptAt": now,
                "delivery.error": null,
                "delivery.nextAttemptAt": null,
            },
            Err(error) if attempts >= MAX_ATTEMPTS => doc! {
                "delivery.status": "failed",
                "delivery.attempts": i64::from(attempts),
                "delivery.lastAttemptAt": now,
                "delivery.error": error,
                "delivery.nextAttemptAt": null,
            },
            Err(error) => doc! {
                "delivery.status": "pending",
                "delivery.attempts": i64::from(attempts),
                "delivery.lastAttemptAt": now,
                "delivery.error": error,
                "delivery.nextAttemptAt": after(now, RETRY_BACKOFF * attempts),
            },
        };

        alerts
            .update_one(doc! { "_id": alert.id }, doc! { "$set": update })
            .await
            .map_err(|e| format!("failed to record delivery of alert {}: {e}", alert.id))?;
    }

    Ok(())
}

fn after(from: DateTime, duration: Duration) -> DateTime {
    DateTime::from_millis(from.timestamp_millis() + duration.as_millis() as i64)
}

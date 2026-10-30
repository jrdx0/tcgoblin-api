use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use mongodb::{
    Database,
    bson::{DateTime, Document, doc, to_bson},
    options::ReturnDocument,
};
use tokio::sync::Notify;

use super::{Extraction, first_text, parse_price};
use crate::{
    db::is_duplicate_key,
    models::{
        alert::{self, Alert, AlertFailure, AlertKind, Delivery, DeliveryStatus},
        check::{self, Check, CheckResult, CheckStatus},
        product::{self, FailureReason, FetchMode, Product, Store, StoreStatus},
        settings::{self, Settings},
    },
    page::PageFetcher,
};

const POLL_INTERVAL: Duration = Duration::from_secs(15);
const LEASE_MARGIN: Duration = Duration::from_secs(5 * 60);
const MAX_TEXT: usize = 500;

pub struct Worker {
    db: Database,
    page: Arc<PageFetcher>,
    alert_signal: Arc<Notify>,
    last_finished: HashMap<String, Instant>,
}

enum Read {
    Text(String),
    Failed(FailureReason, String),
}

struct Observation {
    fetch_mode: FetchMode,
    text: Option<String>,
    outcome: Result<f64, (FailureReason, String)>,
}

impl Worker {
    pub fn new(db: Database, page: Arc<PageFetcher>, alert_signal: Arc<Notify>) -> Self {
        Self {
            db,
            page,
            alert_signal,
            last_finished: HashMap::new(),
        }
    }

    pub async fn run(mut self) {
        loop {
            match self.next().await {
                Ok(true) => {}
                Ok(false) => tokio::time::sleep(POLL_INTERVAL).await,
                Err(err) => {
                    eprintln!("check worker: {err}");
                    tokio::time::sleep(POLL_INTERVAL).await;
                }
            }
        }
    }

    async fn next(&mut self) -> Result<bool, String> {
        let settings = self.settings().await?;
        let timeout = Duration::from_secs(settings.limits.page_timeout_seconds.into());
        let delay = Duration::from_secs(settings.limits.store_delay_seconds.into());

        self.last_finished.retain(|_, at| at.elapsed() < delay);
        let cooling: Vec<&String> = self.last_finished.keys().collect();

        let Some(check) = self.claim(&cooling, timeout * 2 + LEASE_MARGIN).await? else {
            return Ok(false);
        };

        let domain = check.domain.clone();
        let result = self.process(check, &settings, timeout).await;
        self.last_finished.insert(domain, Instant::now());
        result.map(|()| true)
    }

    async fn settings(&self) -> Result<Settings, String> {
        self.db
            .collection::<Settings>(settings::COLLECTION)
            .find_one(doc! { "_id": settings::ID })
            .await
            .map_err(|e| format!("failed to read settings: {e}"))?
            .ok_or_else(|| "settings document is missing".to_string())
    }

    async fn claim(&self, cooling: &[&String], lease: Duration) -> Result<Option<Check>, String> {
        let now = DateTime::now();

        self.db
            .collection::<Check>(check::COLLECTION)
            .find_one_and_update(
                doc! {
                    "scheduledFor": { "$lte": now },
                    "domain": { "$nin": cooling },
                    "$or": [
                        { "status": "pending" },
                        { "status": "running", "lockedUntil": { "$lt": now } },
                    ],
                },
                doc! {
                    "$set": {
                        "status": "running",
                        "lockedUntil": after(now, lease),
                        "startedAt": now,
                    },
                },
            )
            .sort(doc! { "scheduledFor": 1, "attempt": 1 })
            .return_document(ReturnDocument::After)
            .await
            .map_err(|e| format!("failed to claim check: {e}"))
    }

    async fn process(
        &self,
        check: Check,
        settings: &Settings,
        timeout: Duration,
    ) -> Result<(), String> {
        let product = self
            .db
            .collection::<Product>(product::COLLECTION)
            .find_one(doc! { "_id": check.product_id })
            .await
            .map_err(|e| format!("failed to read product: {e}"))?;

        let Some(product) = product else {
            return self.cancel(&check, "product no longer exists").await;
        };
        let Some(store) = product
            .stores
            .iter()
            .find(|s| s.id == check.store_id)
            .cloned()
        else {
            return self.cancel(&check, "store no longer exists").await;
        };
        if store.status == StoreStatus::Paused {
            return self.cancel(&check, "store is paused").await;
        }

        let observation = self.observe(&store, timeout).await;
        let now = DateTime::now();

        let (result, price, error) = match observation.outcome {
            Ok(price) => {
                let result = self
                    .record_price(
                        &product,
                        &store,
                        price,
                        observation.fetch_mode,
                        settings,
                        now,
                    )
                    .await?;
                (result, Some(price), None)
            }
            Err((reason, message)) => {
                self.record_failure(&check, &product, &store, reason, &message, settings, now)
                    .await?;
                (CheckResult::from(reason), None, Some(message))
            }
        };

        self.finish(
            &check,
            doc! {
                "status": to_bson(&CheckStatus::Done).map_err(|e| e.to_string())?,
                "result": to_bson(&result).map_err(|e| e.to_string())?,
                "fetchMode": to_bson(&observation.fetch_mode).map_err(|e| e.to_string())?,
                "text": observation.text.map(|t| truncate(&t, MAX_TEXT)),
                "price": price,
                "error": error,
            },
        )
        .await
    }

    async fn observe(&self, store: &Store, timeout: Duration) -> Observation {
        let (fetch_mode, read) = match store.fetch_mode {
            FetchMode::Browser => (
                FetchMode::Browser,
                self.read(store, FetchMode::Browser, timeout).await,
            ),
            FetchMode::Http => match self.read(store, FetchMode::Http, timeout).await {
                Read::Text(text) => (FetchMode::Http, Read::Text(text)),
                Read::Failed(_, http_error) => {
                    match self.read(store, FetchMode::Browser, timeout).await {
                        Read::Failed(reason, error) => (
                            FetchMode::Browser,
                            Read::Failed(reason, format!("{error} (HTTP attempt: {http_error})")),
                        ),
                        found => (FetchMode::Browser, found),
                    }
                }
            },
        };

        match read {
            Read::Text(text) => {
                let outcome = parse_price(&text).ok_or_else(|| {
                    (
                        FailureReason::InvalidText,
                        format!("could not read a price from \"{}\"", truncate(&text, 100)),
                    )
                });
                Observation {
                    fetch_mode,
                    text: Some(text),
                    outcome,
                }
            }
            Read::Failed(reason, message) => Observation {
                fetch_mode,
                text: None,
                outcome: Err((reason, message)),
            },
        }
    }

    async fn read(&self, store: &Store, mode: FetchMode, timeout: Duration) -> Read {
        let fetched = match mode {
            FetchMode::Http => self.page.fetch_http(&store.url, timeout).await,
            FetchMode::Browser => self.page.fetch_browser(&store.url, timeout).await,
        };
        let page = match fetched {
            Ok(page) => page,
            Err(err) => return Read::Failed(FailureReason::PageUnavailable, err),
        };

        let selector = store.selector.clone();
        match tokio::task::spawn_blocking(move || first_text(&page.html, &selector)).await {
            Ok(Extraction::Found(text)) => Read::Text(text),
            Ok(Extraction::NotFound) => Read::Failed(
                FailureReason::ElementNotFound,
                "selector did not match any element".to_string(),
            ),
            Ok(Extraction::InvalidSelector) => Read::Failed(
                FailureReason::ElementNotFound,
                "selector is not valid CSS".to_string(),
            ),
            Err(err) => Read::Failed(
                FailureReason::PageUnavailable,
                format!("extraction task failed: {err}"),
            ),
        }
    }

    async fn record_price(
        &self,
        product: &Product,
        store: &Store,
        price: f64,
        fetch_mode: FetchMode,
        settings: &Settings,
        now: DateTime,
    ) -> Result<CheckResult, String> {
        let previous = store.last_price();
        let result = match previous {
            None => CheckResult::Initial,
            Some(last) if (last - price).abs() < 0.005 => CheckResult::Unchanged,
            Some(_) => CheckResult::PriceChanged,
        };

        let mut set = doc! {
            "stores.$.status": "ok",
            "stores.$.lastError": null,
            "stores.$.lastSuccessAt": now,
        };
        if fetch_mode != store.fetch_mode {
            set.insert(
                "stores.$.fetchMode",
                to_bson(&fetch_mode).map_err(|e| e.to_string())?,
            );
        }
        let mut update = doc! { "$set": set };
        if result != CheckResult::Unchanged {
            update.insert(
                "$push",
                doc! { "stores.$.priceHistory": { "date": now, "price": price } },
            );
        }
        self.update_store(product, store, update).await?;

        let mut alerts = Vec::new();
        if store.status == StoreStatus::Failing && settings.discord.notify_recovery {
            let mut recovery = new_alert(AlertKind::Recovery, product, store, now);
            recovery.price = Some(price);
            alerts.push(recovery);
        }
        if let (CheckResult::PriceChanged, Some(previous)) = (result, previous) {
            let kind = if price < previous {
                AlertKind::PriceDrop
            } else {
                AlertKind::PriceRise
            };
            let mut change = new_alert(kind, product, store, now);
            change.previous_price = Some(previous);
            change.price = Some(price);
            change.below_average = average(product, store, price).is_some_and(|avg| price < avg);
            alerts.push(change);
        }
        self.insert_alerts(alerts).await?;

        Ok(result)
    }

    #[allow(clippy::too_many_arguments)]
    async fn record_failure(
        &self,
        check: &Check,
        product: &Product,
        store: &Store,
        reason: FailureReason,
        message: &str,
        settings: &Settings,
        now: DateTime,
    ) -> Result<(), String> {
        if check.attempt == 1 {
            let retry_after =
                Duration::from_secs(u64::from(settings.limits.retry_after_minutes) * 60);
            let retry = Check::pending(
                check.product_id,
                check.store_id,
                check.domain.clone(),
                after(now, retry_after),
                check.attempt + 1,
            );
            return match self
                .db
                .collection::<Check>(check::COLLECTION)
                .insert_one(&retry)
                .await
            {
                Ok(_) => Ok(()),
                Err(err) if is_duplicate_key(&err) => Ok(()),
                Err(err) => Err(format!("failed to schedule retry: {err}")),
            };
        }

        let already_failing = store.status == StoreStatus::Failing;
        let since = store
            .last_error
            .as_ref()
            .filter(|_| already_failing)
            .map(|e| e.since)
            .unwrap_or(now);

        self.update_store(
            product,
            store,
            doc! {
                "$set": {
                    "stores.$.status": "failing",
                    "stores.$.lastError": {
                        "date": now,
                        "reason": to_bson(&reason).map_err(|e| e.to_string())?,
                        "since": since,
                    },
                },
            },
        )
        .await?;

        if !already_failing {
            let mut failure = new_alert(AlertKind::Failure, product, store, now);
            failure.failure = Some(AlertFailure {
                reason,
                message: Some(message.to_string()),
                last_success_at: store.last_success_at,
            });
            self.insert_alerts(vec![failure]).await?;
        }

        Ok(())
    }

    async fn update_store(
        &self,
        product: &Product,
        store: &Store,
        update: Document,
    ) -> Result<(), String> {
        self.db
            .collection::<Product>(product::COLLECTION)
            .update_one(doc! { "_id": product.id, "stores.id": store.id }, update)
            .await
            .map(|_| ())
            .map_err(|e| format!("failed to update store {}: {e}", store.id))
    }

    async fn insert_alerts(&self, alerts: Vec<Alert>) -> Result<(), String> {
        if alerts.is_empty() {
            return Ok(());
        }
        self.db
            .collection::<Alert>(alert::COLLECTION)
            .insert_many(alerts)
            .await
            .map_err(|e| format!("failed to insert alerts: {e}"))?;
        self.alert_signal.notify_one();
        Ok(())
    }

    async fn cancel(&self, check: &Check, reason: &str) -> Result<(), String> {
        self.finish(
            check,
            doc! {
                "status": to_bson(&CheckStatus::Cancelled).map_err(|e| e.to_string())?,
                "error": reason,
            },
        )
        .await
    }

    async fn finish(&self, check: &Check, mut set: Document) -> Result<(), String> {
        set.insert("finishedAt", DateTime::now());
        set.insert("lockedUntil", None::<DateTime>);

        self.db
            .collection::<Check>(check::COLLECTION)
            .update_one(doc! { "_id": check.id }, doc! { "$set": set })
            .await
            .map(|_| ())
            .map_err(|e| format!("failed to finish check {}: {e}", check.id))
    }
}

fn average(product: &Product, store: &Store, price: f64) -> Option<f64> {
    let prices: Vec<f64> = product
        .stores
        .iter()
        .filter(|s| s.status != StoreStatus::Paused)
        .filter_map(|s| {
            if s.id == store.id {
                Some(price)
            } else {
                s.last_price()
            }
        })
        .collect();

    (!prices.is_empty()).then(|| prices.iter().sum::<f64>() / prices.len() as f64)
}

fn new_alert(kind: AlertKind, product: &Product, store: &Store, now: DateTime) -> Alert {
    Alert {
        id: mongodb::bson::oid::ObjectId::new(),
        created_at: now,
        kind,
        product_id: product.id,
        product_name: product.name.clone(),
        store_id: store.id,
        store_url: store.url.clone(),
        currency: store.currency.clone(),
        previous_price: None,
        price: None,
        below_average: false,
        failure: None,
        delivery: Delivery {
            status: DeliveryStatus::Pending,
            attempts: 0,
            last_attempt_at: None,
            error: None,
            next_attempt_at: None,
        },
    }
}

fn after(from: DateTime, duration: Duration) -> DateTime {
    DateTime::from_millis(from.timestamp_millis() + duration.as_millis() as i64)
}

fn truncate(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

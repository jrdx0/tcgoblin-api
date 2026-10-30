use std::time::{Duration, Instant};

use chrono::{NaiveTime, TimeDelta, TimeZone, Utc};
use chrono_tz::Tz;
use futures_util::TryStreamExt;
use mongodb::{
    Database,
    bson::{DateTime, doc, to_document},
};

use crate::{
    db::is_duplicate_key,
    models::{
        check::{self, Check},
        product::{self, Product, StoreStatus},
        settings::{self, DailyCheck, Settings},
    },
};

const TICK: Duration = Duration::from_secs(60);
const PRUNE_EVERY: Duration = Duration::from_secs(24 * 60 * 60);
const PRICE_HISTORY_RETENTION: TimeDelta = TimeDelta::days(90);

pub async fn run(db: Database) {
    let mut last_prune: Option<Instant> = None;

    loop {
        match enqueue_due(&db).await {
            Ok(0) => {}
            Ok(queued) => println!("scheduler: queued {queued} checks"),
            Err(err) => eprintln!("scheduler: {err}"),
        }

        if last_prune.is_none_or(|at| at.elapsed() >= PRUNE_EVERY) {
            match prune_price_history(&db).await {
                Ok(0) => {}
                Ok(pruned) => println!("scheduler: pruned price history of {pruned} products"),
                Err(err) => eprintln!("scheduler: {err}"),
            }
            last_prune = Some(Instant::now());
        }

        tokio::time::sleep(TICK).await;
    }
}

pub async fn prune_price_history(db: &Database) -> Result<u64, String> {
    let cutoff = DateTime::from_millis((Utc::now() - PRICE_HISTORY_RETENTION).timestamp_millis());

    let result = db
        .collection::<Product>(product::COLLECTION)
        .update_many(
            doc! { "stores.priceHistory.date": { "$lt": cutoff } },
            doc! { "$pull": { "stores.$[].priceHistory": { "date": { "$lt": cutoff } } } },
        )
        .await
        .map_err(|e| format!("failed to prune price history: {e}"))?;

    Ok(result.modified_count)
}

pub async fn enqueue_due(db: &Database) -> Result<usize, String> {
    let settings = db
        .collection::<Settings>(settings::COLLECTION)
        .find_one(doc! { "_id": settings::ID })
        .await
        .map_err(|e| format!("failed to read settings: {e}"))?
        .ok_or("settings document is missing")?;

    let slot = current_slot(&settings.daily_check)?;

    let products: Vec<Product> = db
        .collection::<Product>(product::COLLECTION)
        .find(doc! {})
        .await
        .map_err(|e| format!("failed to read products: {e}"))?
        .try_collect()
        .await
        .map_err(|e| format!("failed to read products: {e}"))?;

    let checks = db.collection::<Check>(check::COLLECTION);
    let mut queued = 0;

    for product in &products {
        for store in product
            .stores
            .iter()
            .filter(|s| s.status != StoreStatus::Paused)
        {
            let check = Check::pending(product.id, store.id, store.domain(), slot, 1);
            let document = to_document(&check).map_err(|e| e.to_string())?;

            let result = checks
                .update_one(
                    doc! { "storeId": store.id, "scheduledFor": slot, "attempt": 1 },
                    doc! { "$setOnInsert": document },
                )
                .upsert(true)
                .await;

            match result {
                Ok(r) if r.upserted_id.is_some() => queued += 1,
                Ok(_) => {}
                Err(err) if is_duplicate_key(&err) => {}
                Err(err) => {
                    return Err(format!(
                        "failed to queue check for store {}: {err}",
                        store.id
                    ));
                }
            }
        }
    }

    Ok(queued)
}

fn current_slot(daily_check: &DailyCheck) -> Result<DateTime, String> {
    let tz: Tz = daily_check
        .timezone
        .parse()
        .map_err(|_| format!("invalid timezone {}", daily_check.timezone))?;
    let time = NaiveTime::parse_from_str(&daily_check.time, "%H:%M")
        .map_err(|_| format!("invalid daily check time {}", daily_check.time))?;

    let now = Utc::now().with_timezone(&tz);
    let today = now.date_naive();

    let at = |date: chrono::NaiveDate| {
        let local = date.and_time(time);
        tz.from_local_datetime(&local).earliest().or_else(|| {
            tz.from_local_datetime(&(local + TimeDelta::hours(1)))
                .earliest()
        })
    };

    let slot = match at(today) {
        Some(slot) if slot <= now => slot,
        _ => at(today - TimeDelta::days(1)).ok_or("could not resolve the previous daily check")?,
    };

    Ok(DateTime::from_millis(slot.timestamp_millis()))
}

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use mongodb::bson::{DateTime, doc};
use serde::Serialize;

use crate::{
    AppState,
    error::AppError,
    models::{
        check::{self, Check, CheckResult, CheckStatus},
        product::{self, FetchMode, Product, StoreStatus},
    },
    routes::products::{parse_product_id, parse_store_id, store_not_found, to_rfc3339},
};

pub async fn run_now(
    State(state): State<AppState>,
    Path((product_id, store_id)): Path<(String, String)>,
) -> Result<(StatusCode, Json<CheckResponse>), AppError> {
    let product_oid = parse_product_id(&product_id)?;
    let store_oid = parse_store_id(&product_id, &store_id)?;

    let store = state
        .db
        .collection::<Product>(product::COLLECTION)
        .find_one(doc! { "_id": product_oid, "stores.id": store_oid })
        .await?
        .and_then(|p| p.stores.into_iter().find(|s| s.id == store_oid))
        .ok_or_else(|| store_not_found(&product_id, &store_id))?;

    if store.status == StoreStatus::Paused {
        return Err(AppError::Validation(
            "store is paused; resume it before running a check".to_string(),
        ));
    }

    let checks = state.db.collection::<Check>(check::COLLECTION);
    let now = DateTime::now();

    let in_progress = checks
        .find_one(doc! {
            "storeId": store_oid,
            "status": { "$in": ["pending", "running"] },
            "scheduledFor": { "$lte": now },
        })
        .await?;
    if let Some(existing) = in_progress {
        return Ok((StatusCode::OK, Json(existing.into())));
    }

    let check = Check::pending(product_oid, store_oid, store.domain(), now, 1);
    checks.insert_one(&check).await?;

    Ok((StatusCode::ACCEPTED, Json(check.into())))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResponse {
    id: String,
    product_id: String,
    store_id: String,
    scheduled_for: String,
    attempt: u32,
    status: CheckStatus,
    started_at: Option<String>,
    finished_at: Option<String>,
    result: Option<CheckResult>,
    fetch_mode: Option<FetchMode>,
    text: Option<String>,
    price: Option<f64>,
    error: Option<String>,
}

impl From<Check> for CheckResponse {
    fn from(c: Check) -> Self {
        Self {
            id: c.id.to_hex(),
            product_id: c.product_id.to_hex(),
            store_id: c.store_id.to_hex(),
            scheduled_for: to_rfc3339(c.scheduled_for),
            attempt: c.attempt,
            status: c.status,
            started_at: c.started_at.map(to_rfc3339),
            finished_at: c.finished_at.map(to_rfc3339),
            result: c.result,
            fetch_mode: c.fetch_mode,
            text: c.text,
            price: c.price,
            error: c.error,
        }
    }
}

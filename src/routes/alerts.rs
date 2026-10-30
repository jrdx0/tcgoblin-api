use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use futures_util::TryStreamExt;
use mongodb::bson::{DateTime, doc, oid::ObjectId};
use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    AppState,
    error::AppError,
    models::{
        alert::{self, Alert, AlertFailure, AlertKind, Delivery, DeliveryStatus},
        product::{self, FailureReason, Product},
    },
    routes::products::{to_rfc3339, validate_url},
};

const DEFAULT_LIMIT: u32 = 100;
const MAX_LIMIT: u32 = 500;

#[derive(Deserialize)]
pub struct ListAlerts {
    limit: Option<u32>,
}

pub async fn list(
    State(state): State<AppState>,
    Query(query): Query<ListAlerts>,
) -> Result<Json<Vec<AlertResponse>>, AppError> {
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT);
    if limit == 0 || limit > MAX_LIMIT {
        return Err(AppError::Validation(format!(
            "limit must be between 1 and {MAX_LIMIT}"
        )));
    }

    let alerts: Vec<Alert> = state
        .db
        .collection::<Alert>(alert::COLLECTION)
        .find(doc! {})
        .sort(doc! { "createdAt": -1, "_id": -1 })
        .limit(i64::from(limit))
        .await?
        .try_collect()
        .await?;

    Ok(Json(alerts.into_iter().map(Into::into).collect()))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TestAlert {
    kind: Option<AlertKind>,
    created_at: Option<String>,
    product_id: Option<String>,
    product_name: Option<String>,
    store_id: Option<String>,
    store_url: Option<String>,
    currency: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    previous_price: Option<Option<f64>>,
    #[serde(default, deserialize_with = "nullable")]
    price: Option<Option<f64>>,
    below_average: Option<bool>,
    #[serde(default, deserialize_with = "nullable")]
    failure: Option<Option<TestFailure>>,
    delivery: Option<TestDelivery>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestFailure {
    reason: Option<FailureReason>,
    #[serde(default, deserialize_with = "nullable")]
    message: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    last_success_at: Option<Option<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestDelivery {
    status: Option<DeliveryStatus>,
    attempts: Option<u32>,
    #[serde(default, deserialize_with = "nullable")]
    last_attempt_at: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable")]
    error: Option<Option<String>>,
}

fn nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

pub async fn create_test(
    State(state): State<AppState>,
    body: Option<Json<TestAlert>>,
) -> Result<(StatusCode, Json<AlertResponse>), AppError> {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let kind = body.kind.unwrap_or(AlertKind::PriceDrop);

    let product_oid = body
        .product_id
        .as_deref()
        .map(|id| parse_id("productId", id))
        .transpose()?;
    let store_oid = body
        .store_id
        .as_deref()
        .map(|id| parse_id("storeId", id))
        .transpose()?;

    let filter = match product_oid {
        Some(id) => doc! { "_id": id },
        None => doc! { "stores.0": { "$exists": true } },
    };
    let product = state
        .db
        .collection::<Product>(product::COLLECTION)
        .find_one(filter)
        .await?;
    let target = product.and_then(|p| {
        let store = match store_oid {
            Some(id) => p.stores.into_iter().find(|s| s.id == id),
            None => p.stores.into_iter().next(),
        }?;
        Some((p.id, p.name, store.id, store.url, store.currency))
    });
    let (product_id, product_name, store_id, store_url, currency) = target.unwrap_or_else(|| {
        (
            ObjectId::new(),
            "Starter deck ST01".to_string(),
            ObjectId::new(),
            "https://tiendados.example/gundam-st01-starter-deck".to_string(),
            "MXN".to_string(),
        )
    });

    let now = DateTime::now();
    let created_at = match body.created_at.as_deref() {
        Some(date) => parse_date("createdAt", date)?,
        None => now,
    };

    let (previous_price, price, below_average, failure) = match kind {
        AlertKind::PriceDrop => (Some(289.0), Some(259.0), true, None),
        AlertKind::PriceRise => (Some(259.0), Some(289.0), false, None),
        AlertKind::Recovery => (None, Some(259.0), false, None),
        AlertKind::Failure => (
            None,
            None,
            false,
            Some(AlertFailure {
                reason: FailureReason::ElementNotFound,
                message: Some("selector did not match any element".to_string()),
                last_success_at: Some(DateTime::from_millis(
                    created_at.timestamp_millis() - 24 * 60 * 60 * 1000,
                )),
            }),
        ),
    };

    let failure = match body.failure {
        None => failure,
        Some(None) => None,
        Some(Some(input)) => {
            let base = failure.unwrap_or(AlertFailure {
                reason: FailureReason::ElementNotFound,
                message: None,
                last_success_at: None,
            });
            Some(AlertFailure {
                reason: input.reason.unwrap_or(base.reason),
                message: input.message.unwrap_or(base.message),
                last_success_at: match input.last_success_at {
                    None => base.last_success_at,
                    Some(None) => None,
                    Some(Some(date)) => Some(parse_date("failure.lastSuccessAt", &date)?),
                },
            })
        }
    };

    let delivery = match body.delivery {
        None => Delivery {
            status: DeliveryStatus::Pending,
            attempts: 0,
            last_attempt_at: None,
            error: None,
            next_attempt_at: None,
        },
        Some(input) => Delivery {
            status: input.status.unwrap_or(DeliveryStatus::Pending),
            attempts: input.attempts.unwrap_or(0),
            last_attempt_at: match input.last_attempt_at.flatten() {
                Some(date) => Some(parse_date("delivery.lastAttemptAt", &date)?),
                None => None,
            },
            error: input.error.flatten(),
            next_attempt_at: None,
        },
    };

    let alert = Alert {
        id: ObjectId::new(),
        created_at,
        kind,
        product_id: product_oid.unwrap_or(product_id),
        product_name: body.product_name.unwrap_or(product_name),
        store_id: store_oid.unwrap_or(store_id),
        store_url: match body.store_url {
            Some(url) => validate_url(&url)?,
            None => store_url,
        },
        currency: body.currency.unwrap_or(currency),
        previous_price: body.previous_price.unwrap_or(previous_price),
        price: body.price.unwrap_or(price),
        below_average: body.below_average.unwrap_or(below_average),
        failure,
        delivery,
    };

    state
        .db
        .collection::<Alert>(alert::COLLECTION)
        .insert_one(&alert)
        .await?;

    Ok((StatusCode::CREATED, Json(alert.into())))
}

#[derive(Serialize)]
pub struct DeleteAllResponse {
    deleted: u64,
}

pub async fn delete_all(
    State(state): State<AppState>,
) -> Result<Json<DeleteAllResponse>, AppError> {
    let result = state
        .db
        .collection::<Alert>(alert::COLLECTION)
        .delete_many(doc! {})
        .await?;

    Ok(Json(DeleteAllResponse {
        deleted: result.deleted_count,
    }))
}

pub async fn delete(
    State(state): State<AppState>,
    Path(alert_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let not_found = || AppError::NotFound(format!("alert {alert_id} not found"));
    let alert_oid = ObjectId::parse_str(&alert_id).map_err(|_| not_found())?;

    let result = state
        .db
        .collection::<Alert>(alert::COLLECTION)
        .delete_one(doc! { "_id": alert_oid })
        .await?;

    if result.deleted_count == 0 {
        return Err(not_found());
    }

    Ok(StatusCode::NO_CONTENT)
}

fn parse_id(field: &str, value: &str) -> Result<ObjectId, AppError> {
    ObjectId::parse_str(value)
        .map_err(|_| AppError::Validation(format!("{field} must be a valid ObjectId")))
}

fn parse_date(field: &str, value: &str) -> Result<DateTime, AppError> {
    DateTime::parse_rfc3339_str(value)
        .map_err(|_| AppError::Validation(format!("{field} must be an RFC 3339 date")))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertResponse {
    id: String,
    created_at: String,
    kind: AlertKind,
    product_id: String,
    product_name: String,
    store_id: String,
    store_url: String,
    currency: String,
    previous_price: Option<f64>,
    price: Option<f64>,
    below_average: bool,
    failure: Option<AlertFailureResponse>,
    delivery: DeliveryResponse,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertFailureResponse {
    reason: FailureReason,
    message: Option<String>,
    last_success_at: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryResponse {
    status: DeliveryStatus,
    attempts: u32,
    last_attempt_at: Option<String>,
    error: Option<String>,
    next_attempt_at: Option<String>,
}

impl From<Alert> for AlertResponse {
    fn from(a: Alert) -> Self {
        Self {
            id: a.id.to_hex(),
            created_at: to_rfc3339(a.created_at),
            kind: a.kind,
            product_id: a.product_id.to_hex(),
            product_name: a.product_name,
            store_id: a.store_id.to_hex(),
            store_url: a.store_url,
            currency: a.currency,
            previous_price: a.previous_price,
            price: a.price,
            below_average: a.below_average,
            failure: a.failure.map(Into::into),
            delivery: a.delivery.into(),
        }
    }
}

impl From<AlertFailure> for AlertFailureResponse {
    fn from(f: AlertFailure) -> Self {
        Self {
            reason: f.reason,
            message: f.message,
            last_success_at: f.last_success_at.map(to_rfc3339),
        }
    }
}

impl From<Delivery> for DeliveryResponse {
    fn from(d: Delivery) -> Self {
        Self {
            status: d.status,
            attempts: d.attempts,
            last_attempt_at: d.last_attempt_at.map(to_rfc3339),
            error: d.error,
            next_attempt_at: d.next_attempt_at.map(to_rfc3339),
        }
    }
}

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use futures_util::TryStreamExt;
use mongodb::{
    bson::{DateTime, doc, oid::ObjectId, to_bson},
    options::ReturnDocument,
};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    AppState,
    error::AppError,
    models::product::{
        self, FailureReason, FetchMode, LastError, PricePoint, Product, Store, StoreStatus,
    },
};

pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<ProductResponse>>, AppError> {
    let products: Vec<Product> = state
        .db
        .collection::<Product>(product::COLLECTION)
        .find(doc! {})
        .sort(doc! { "name": 1 })
        .await?
        .try_collect()
        .await?;

    Ok(Json(products.into_iter().map(Into::into).collect()))
}

pub async fn get(
    State(state): State<AppState>,
    Path(product_id): Path<String>,
) -> Result<Json<ProductResponse>, AppError> {
    let product_oid = parse_product_id(&product_id)?;

    let product = state
        .db
        .collection::<Product>(product::COLLECTION)
        .find_one(doc! { "_id": product_oid })
        .await?
        .ok_or_else(|| product_not_found(&product_id))?;

    Ok(Json(product.into()))
}

#[derive(Deserialize)]
pub struct CreateProduct {
    name: String,
    notes: Option<String>,
    #[serde(default)]
    stores: Vec<AddStore>,
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<CreateProduct>,
) -> Result<(StatusCode, Json<ProductResponse>), AppError> {
    let name = validate_name(&body.name)?;
    let notes = body.notes.and_then(normalize_notes);

    let now = DateTime::now();
    let stores = body
        .stores
        .into_iter()
        .enumerate()
        .map(|(i, input)| {
            build_store(input, now).map_err(|err| match err {
                AppError::Validation(message) => {
                    AppError::Validation(format!("stores[{i}]: {message}"))
                }
                other => other,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let product = Product {
        id: ObjectId::new(),
        name,
        notes,
        is_favorite: false,
        created_at: now,
        updated_at: now,
        stores,
    };

    state
        .db
        .collection::<Product>(product::COLLECTION)
        .insert_one(&product)
        .await?;

    Ok((StatusCode::CREATED, Json(product.into())))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddStore {
    url: String,
    selector: String,
    currency: Option<String>,
    fetch_mode: Option<FetchMode>,
}

pub async fn add_store(
    State(state): State<AppState>,
    Path(product_id): Path<String>,
    Json(body): Json<AddStore>,
) -> Result<(StatusCode, Json<StoreResponse>), AppError> {
    let product_oid = parse_product_id(&product_id)?;

    let now = DateTime::now();
    let store = build_store(body, now)?;

    let result = state
        .db
        .collection::<Product>(product::COLLECTION)
        .update_one(
            doc! { "_id": product_oid },
            doc! {
                "$push": { "stores": to_bson(&store)? },
                "$set": { "updatedAt": now },
            },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(product_not_found(&product_id));
    }

    Ok((StatusCode::CREATED, Json(store.into())))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProduct {
    name: Option<String>,
    notes: Option<String>,
    is_favorite: Option<bool>,
}

pub async fn update(
    State(state): State<AppState>,
    Path(product_id): Path<String>,
    Json(body): Json<UpdateProduct>,
) -> Result<Json<ProductResponse>, AppError> {
    let product_oid = parse_product_id(&product_id)?;

    let mut set = doc! { "updatedAt": DateTime::now() };
    if let Some(name) = body.name {
        set.insert("name", validate_name(&name)?);
    }
    if let Some(notes) = body.notes {
        set.insert("notes", normalize_notes(notes));
    }
    if let Some(is_favorite) = body.is_favorite {
        set.insert("isFavorite", is_favorite);
    }

    let product = state
        .db
        .collection::<Product>(product::COLLECTION)
        .find_one_and_update(doc! { "_id": product_oid }, doc! { "$set": set })
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| product_not_found(&product_id))?;

    Ok(Json(product.into()))
}

pub async fn delete(
    State(state): State<AppState>,
    Path(product_id): Path<String>,
) -> Result<StatusCode, AppError> {
    let product_oid = parse_product_id(&product_id)?;

    let result = state
        .db
        .collection::<Product>(product::COLLECTION)
        .delete_one(doc! { "_id": product_oid })
        .await?;

    if result.deleted_count == 0 {
        return Err(product_not_found(&product_id));
    }

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStore {
    url: Option<String>,
    selector: Option<String>,
    currency: Option<String>,
    fetch_mode: Option<FetchMode>,
    status: Option<StoreStatus>,
}

pub async fn update_store(
    State(state): State<AppState>,
    Path((product_id, store_id)): Path<(String, String)>,
    Json(body): Json<UpdateStore>,
) -> Result<Json<StoreResponse>, AppError> {
    let product_oid = parse_product_id(&product_id)?;
    let store_oid = parse_store_id(&product_id, &store_id)?;

    let mut set = doc! { "updatedAt": DateTime::now() };
    if let Some(url) = body.url {
        set.insert("stores.$.url", validate_url(&url)?);
    }
    if let Some(selector) = body.selector {
        set.insert("stores.$.selector", validate_selector(&selector)?);
    }
    if let Some(currency) = body.currency {
        set.insert("stores.$.currency", validate_currency(&currency)?);
    }
    if let Some(fetch_mode) = body.fetch_mode {
        set.insert("stores.$.fetchMode", to_bson(&fetch_mode)?);
    }
    if let Some(status) = body.status {
        set.insert("stores.$.status", to_bson(&validate_status(status)?)?);
    }

    let product = state
        .db
        .collection::<Product>(product::COLLECTION)
        .find_one_and_update(
            doc! { "_id": product_oid, "stores.id": store_oid },
            doc! { "$set": set },
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| store_not_found(&product_id, &store_id))?;

    let store = product
        .stores
        .into_iter()
        .find(|s| s.id == store_oid)
        .ok_or_else(|| store_not_found(&product_id, &store_id))?;

    Ok(Json(store.into()))
}

pub async fn delete_store(
    State(state): State<AppState>,
    Path((product_id, store_id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    let product_oid = parse_product_id(&product_id)?;
    let store_oid = parse_store_id(&product_id, &store_id)?;

    let result = state
        .db
        .collection::<Product>(product::COLLECTION)
        .update_one(
            doc! { "_id": product_oid, "stores.id": store_oid },
            doc! {
                "$pull": { "stores": { "id": store_oid } },
                "$set": { "updatedAt": DateTime::now() },
            },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(store_not_found(&product_id, &store_id));
    }

    Ok(StatusCode::NO_CONTENT)
}

pub(crate) fn parse_product_id(product_id: &str) -> Result<ObjectId, AppError> {
    ObjectId::parse_str(product_id).map_err(|_| product_not_found(product_id))
}

pub(crate) fn parse_store_id(product_id: &str, store_id: &str) -> Result<ObjectId, AppError> {
    ObjectId::parse_str(store_id).map_err(|_| store_not_found(product_id, store_id))
}

fn product_not_found(product_id: &str) -> AppError {
    AppError::NotFound(format!("product {product_id} not found"))
}

pub(crate) fn store_not_found(product_id: &str, store_id: &str) -> AppError {
    AppError::NotFound(format!(
        "store {store_id} not found in product {product_id}"
    ))
}

fn build_store(body: AddStore, now: DateTime) -> Result<Store, AppError> {
    Ok(Store {
        id: ObjectId::new(),
        url: validate_url(&body.url)?,
        selector: validate_selector(&body.selector)?,
        currency: match body.currency {
            Some(currency) => validate_currency(&currency)?,
            None => "MXN".to_string(),
        },
        fetch_mode: body.fetch_mode.unwrap_or(FetchMode::Browser),
        status: StoreStatus::Ok,
        last_error: None,
        last_success_at: None,
        created_at: now,
        price_history: Vec::new(),
    })
}

fn validate_name(name: &str) -> Result<String, AppError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Validation("name must not be empty".to_string()));
    }
    Ok(name.to_string())
}

fn normalize_notes(notes: String) -> Option<String> {
    let notes = notes.trim();
    (!notes.is_empty()).then(|| notes.to_string())
}

pub(crate) fn validate_url(url: &str) -> Result<String, AppError> {
    Url::parse(url.trim())
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https"))
        .map(|u| u.to_string())
        .ok_or_else(|| AppError::Validation("url must be a valid http(s) URL".to_string()))
}

fn validate_selector(selector: &str) -> Result<String, AppError> {
    let selector = selector.trim();
    if selector.is_empty() {
        return Err(AppError::Validation(
            "selector must not be empty".to_string(),
        ));
    }
    Ok(selector.to_string())
}

fn validate_currency(currency: &str) -> Result<String, AppError> {
    let currency = currency.trim().to_uppercase();
    if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(AppError::Validation(
            "currency must be a 3-letter ISO code".to_string(),
        ));
    }
    Ok(currency)
}

fn validate_status(status: StoreStatus) -> Result<StoreStatus, AppError> {
    match status {
        StoreStatus::Ok | StoreStatus::Paused => Ok(status),
        StoreStatus::Failing => Err(AppError::Validation(
            "status must be \"paused\" or \"ok\"".to_string(),
        )),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProductResponse {
    id: String,
    name: String,
    notes: Option<String>,
    is_favorite: bool,
    created_at: String,
    updated_at: String,
    stores: Vec<StoreResponse>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreResponse {
    id: String,
    url: String,
    selector: String,
    currency: String,
    fetch_mode: FetchMode,
    status: StoreStatus,
    last_error: Option<LastErrorResponse>,
    last_success_at: Option<String>,
    created_at: String,
    price_history: Vec<PricePointResponse>,
}

#[derive(Serialize)]
pub struct LastErrorResponse {
    date: String,
    reason: FailureReason,
    since: String,
}

#[derive(Serialize)]
pub struct PricePointResponse {
    date: String,
    price: f64,
}

pub(crate) fn to_rfc3339(date: DateTime) -> String {
    date.try_to_rfc3339_string()
        .unwrap_or_else(|_| date.timestamp_millis().to_string())
}

impl From<Product> for ProductResponse {
    fn from(p: Product) -> Self {
        Self {
            id: p.id.to_hex(),
            name: p.name,
            notes: p.notes,
            is_favorite: p.is_favorite,
            created_at: to_rfc3339(p.created_at),
            updated_at: to_rfc3339(p.updated_at),
            stores: p.stores.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<Store> for StoreResponse {
    fn from(s: Store) -> Self {
        Self {
            id: s.id.to_hex(),
            url: s.url,
            selector: s.selector,
            currency: s.currency,
            fetch_mode: s.fetch_mode,
            status: s.status,
            last_error: s.last_error.map(Into::into),
            last_success_at: s.last_success_at.map(to_rfc3339),
            created_at: to_rfc3339(s.created_at),
            price_history: s.price_history.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<LastError> for LastErrorResponse {
    fn from(e: LastError) -> Self {
        Self {
            date: to_rfc3339(e.date),
            reason: e.reason,
            since: to_rfc3339(e.since),
        }
    }
}

impl From<PricePoint> for PricePointResponse {
    fn from(p: PricePoint) -> Self {
        Self {
            date: to_rfc3339(p.date),
            price: p.price,
        }
    }
}

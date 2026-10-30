use std::time::Duration;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use mongodb::bson::{DateTime, doc};
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    error::AppError,
    models::{
        product::FetchMode,
        settings::{self, Settings},
    },
    routes::products::{to_rfc3339, validate_url},
};

#[derive(Deserialize)]
pub struct SnapshotRequest {
    url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotQuery {
    fetch_mode: Option<FetchMode>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotResponse {
    url: String,
    html: String,
    fetch_mode: FetchMode,
    taken_at: String,
}

pub async fn snapshot(
    State(state): State<AppState>,
    Query(query): Query<SnapshotQuery>,
    Json(body): Json<SnapshotRequest>,
) -> Response {
    let url = match validate_url(&body.url) {
        Ok(url) => url,
        Err(err) => return err.into_response(),
    };

    let timeout = match page_timeout(&state).await {
        Ok(timeout) => timeout,
        Err(err) => return err.into_response(),
    };

    match state.page.snapshot(&url, timeout, query.fetch_mode).await {
        Ok(snapshot) => Json(SnapshotResponse {
            url: snapshot.url,
            html: snapshot.html,
            fetch_mode: snapshot.fetch_mode,
            taken_at: to_rfc3339(DateTime::now()),
        })
        .into_response(),
        Err(message) => (StatusCode::UNPROCESSABLE_ENTITY, message).into_response(),
    }
}

async fn page_timeout(state: &AppState) -> Result<Duration, AppError> {
    let settings = state
        .db
        .collection::<Settings>(settings::COLLECTION)
        .find_one(doc! { "_id": settings::ID })
        .await?
        .ok_or_else(|| AppError::Internal("settings document is missing".to_string()))?;

    Ok(Duration::from_secs(
        settings.limits.page_timeout_seconds.into(),
    ))
}

use std::sync::Arc;

use axum::{
    Router,
    http::{HeaderValue, Method, header},
    routing::{delete, get, patch, post, put},
};
use tcgoblin_api::{AppState, config, db, discord::DiscordClient, routes};
use tower_http::cors::CorsLayer;

async fn healthy() -> &'static str {
    "OK"
}

fn cors_layer() -> CorsLayer {
    let origins: Vec<HeaderValue> = config::env_var("CORS_ALLOWED_ORIGINS")
        .split(',')
        .map(str::trim)
        .filter(|o| !o.is_empty())
        .map(|o| {
            o.parse()
                .unwrap_or_else(|_| panic!("invalid origin in CORS_ALLOWED_ORIGINS: {o}"))
        })
        .collect();

    CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([header::CONTENT_TYPE])
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let state = AppState {
        db: db::connect().await,
        page: config::page_fetcher(),
        cipher: Arc::new(config::cipher()),
        discord: DiscordClient::new(),
    };

    let app = Router::new()
        .route("/healthy", get(healthy))
        .route("/product", post(routes::products::create))
        .route(
            "/product/{id}",
            get(routes::products::get)
                .patch(routes::products::update)
                .delete(routes::products::delete),
        )
        .route("/product/{id}/store", post(routes::products::add_store))
        .route(
            "/product/{id}/store/{store_id}",
            patch(routes::products::update_store).delete(routes::products::delete_store),
        )
        .route(
            "/product/{id}/store/{store_id}/check",
            post(routes::checks::run_now),
        )
        .route("/products", get(routes::products::list))
        .route(
            "/alerts",
            get(routes::alerts::list).delete(routes::alerts::delete_all),
        )
        .route("/alerts/test", post(routes::alerts::create_test))
        .route("/alert/{id}", delete(routes::alerts::delete))
        .route("/page/snapshot", post(routes::page::snapshot))
        .route(
            "/settings",
            get(routes::settings::get).put(routes::settings::update),
        )
        .route("/settings/webhook", put(routes::settings::update_webhook))
        .route(
            "/settings/webhook/test",
            post(routes::settings::test_webhook),
        )
        .layer(cors_layer())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

pub mod alerts;
pub mod checks;
pub mod config;
pub mod crypto;
pub mod db;
pub mod discord;
pub mod error;
pub mod models;
pub mod page;
pub mod routes;
pub mod scheduler;

use std::sync::Arc;

use mongodb::Database;

use crypto::Cipher;
use discord::DiscordClient;
use page::PageFetcher;

#[derive(Clone)]
pub struct AppState {
    pub db: Database,
    pub page: Arc<PageFetcher>,
    pub cipher: Arc<Cipher>,
    pub discord: DiscordClient,
}

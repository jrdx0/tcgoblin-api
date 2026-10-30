use std::sync::Arc;

use tcgoblin_api::{alerts, checks::worker::Worker, config, db, discord::DiscordClient};
use tokio::sync::Notify;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let db = db::connect().await;
    let page = config::page_fetcher();
    let alert_signal = Arc::new(Notify::new());

    tokio::spawn(alerts::dispatcher::run(
        db.clone(),
        Arc::new(config::cipher()),
        DiscordClient::new(),
        alert_signal.clone(),
    ));

    Worker::new(db, page, alert_signal).run().await;
}

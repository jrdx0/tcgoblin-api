use tcgoblin_api::{db, scheduler};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let db = db::connect().await;
    scheduler::run(db).await;
}

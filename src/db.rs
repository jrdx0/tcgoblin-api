use mongodb::{
    Client, Database, IndexModel,
    bson::{DateTime, Document, doc},
    error::{Error, ErrorKind, WriteFailure},
    options::{ClientOptions, Credential, IndexOptions, ServerAddress},
};

use crate::{
    config::env_var,
    models::{alert, check, product, settings},
};

pub async fn connect() -> Database {
    let host = env_var("MONGO_HOST");
    let port = env_var("MONGO_PORT")
        .parse()
        .expect("MONGO_PORT must be a valid port");

    let credential = Credential::builder()
        .username(env_var("MONGO_INITDB_ROOT_USERNAME"))
        .password(env_var("MONGO_INITDB_ROOT_PASSWORD"))
        .source("admin".to_string())
        .build();

    let options = ClientOptions::builder()
        .hosts(vec![ServerAddress::Tcp {
            host,
            port: Some(port),
        }])
        .credential(credential)
        .build();

    let client = Client::with_options(options).expect("invalid MongoDB options");
    let db = client.database(&env_var("MONGO_INITDB_DATABASE"));

    db.run_command(doc! { "ping": 1 })
        .await
        .expect("failed to connect to MongoDB");

    ensure_collections(&db).await;
    ensure_settings(&db).await;
    ensure_indexes(&db).await;

    db
}

pub fn is_duplicate_key(err: &Error) -> bool {
    matches!(
        err.kind.as_ref(),
        ErrorKind::Write(WriteFailure::WriteError(e)) if e.code == 11000
    )
}

async fn ensure_collections(db: &Database) {
    let existing = db
        .list_collection_names()
        .await
        .expect("failed to list MongoDB collections");

    for name in [
        product::COLLECTION,
        settings::COLLECTION,
        alert::COLLECTION,
        check::COLLECTION,
    ] {
        if !existing.iter().any(|c| c == name) {
            db.create_collection(name)
                .await
                .unwrap_or_else(|e| panic!("failed to create collection {name}: {e}"));
        }
    }
}

async fn ensure_settings(db: &Database) {
    let collection = db.collection::<settings::Settings>(settings::COLLECTION);
    let exists = collection
        .find_one(doc! { "_id": settings::ID })
        .await
        .expect("failed to read settings")
        .is_some();

    if !exists {
        collection
            .insert_one(settings::Settings::defaults(DateTime::now()))
            .await
            .expect("failed to create default settings");
    }
}

async fn ensure_indexes(db: &Database) {
    let ttl = IndexOptions::builder()
        .name(alert::TTL_INDEX.to_string())
        .expire_after(alert::TTL)
        .build();
    create_index(db, alert::COLLECTION, doc! { "createdAt": 1 }, ttl).await;

    create_index(
        db,
        alert::COLLECTION,
        doc! { "delivery.status": 1, "delivery.nextAttemptAt": 1 },
        named("alerts_delivery"),
    )
    .await;

    let slot = IndexOptions::builder()
        .name("checks_slot".to_string())
        .unique(true)
        .build();
    create_index(
        db,
        check::COLLECTION,
        doc! { "storeId": 1, "scheduledFor": 1, "attempt": 1 },
        slot,
    )
    .await;

    create_index(
        db,
        check::COLLECTION,
        doc! { "status": 1, "scheduledFor": 1 },
        named("checks_queue"),
    )
    .await;

    let checks_ttl = IndexOptions::builder()
        .name(check::TTL_INDEX.to_string())
        .expire_after(check::TTL)
        .build();
    create_index(db, check::COLLECTION, doc! { "finishedAt": 1 }, checks_ttl).await;
}

fn named(name: &str) -> IndexOptions {
    IndexOptions::builder().name(name.to_string()).build()
}

async fn create_index(db: &Database, collection: &str, keys: Document, options: IndexOptions) {
    let name = options.name.clone().unwrap_or_default();
    let index = IndexModel::builder().keys(keys).options(options).build();

    db.collection::<Document>(collection)
        .create_index(index)
        .await
        .unwrap_or_else(|e| panic!("failed to create index {name} on {collection}: {e}"));
}

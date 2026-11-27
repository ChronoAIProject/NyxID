use super::*;
use crate::models::platform_entry_identity::NATIVE_RECEIPTS;
use futures::TryStreamExt;

/// This test is run by both the pre-8b and 8b test executables against the
/// same database to verify that every restart preserves the original TTL.
#[tokio::test]
#[ignore = "run the base and candidate test executables in both restart orders"]
async fn mixed_fleet_receipt_index_restart() {
    let database_name = std::env::var("NYXID_MIXED_FLEET_DATABASE").unwrap();
    let uri = std::env::var("NYXID_TEST_DATABASE_URL").unwrap();
    let client = mongodb::Client::with_uri_str(uri).await.unwrap();
    let db = client.database(&database_name);

    ensure_indexes(&db).await.unwrap();

    let indexes = db
        .collection::<bson::Document>(NATIVE_RECEIPTS)
        .list_indexes()
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    let expiry = indexes
        .iter()
        .find(|index| {
            index
                .options
                .as_ref()
                .and_then(|options| options.name.as_deref())
                == Some("platform_native_receipt_expiry_v2")
        })
        .unwrap();
    assert_eq!(expiry.keys, doc! {"expires_at": 1});
    assert_eq!(
        expiry.options.as_ref().unwrap().expire_after,
        Some(std::time::Duration::ZERO)
    );
}

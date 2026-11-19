use super::*;
use mongodb::bson::{self, Document, doc};

#[tokio::test]
async fn integrity_api_preserves_severity_and_details_including_legacy_rows() {
    let db = crate::test_utils::connect_transaction_test_database("integrity_api_severity").await;
    let at = bson::DateTime::from_millis(0);
    let mut legacy = doc! {
        "_id":"legacy", "active":true, "code":"exact_service_semantic_effect_index",
        "summary":"Existing diagnostic", "detail":"Existing detail", "remediation":"Review",
        "detected_at":at, "updated_at":at,
    };
    let mut information = legacy.clone();
    information.insert("_id", "information");
    information.insert("code", "test_information");
    information.insert("severity", "info");
    information.insert("technical_details", "complete=true");
    db.collection::<Document>("startup_diagnostics")
        .insert_many([legacy.clone(), information])
        .await
        .unwrap();
    let organization = "00000000-0000-4000-8000-000000000001";
    db.collection::<Document>("users")
        .insert_one(doc! {
            "_id":organization, "user_type":"org", "deletion_audit":{"requested_at":at}
        })
        .await
        .unwrap();
    let result = serde_json::to_value(active_startup_diagnostics(&db).await.unwrap()).unwrap();
    let entries = result.as_array().unwrap();
    let entry = |code: &str| entries.iter().find(|item| item["code"] == code).unwrap();
    assert_eq!(
        entry("exact_service_semantic_effect_index")["severity"],
        "action"
    );
    assert_eq!(
        entry("organization_deletion_incomplete")["severity"],
        "action"
    );
    assert_eq!(entry("test_information")["severity"], "info");
    assert_eq!(
        entry("test_information")["technical_details"],
        "complete=true"
    );
    for item in entries {
        assert!(matches!(item["severity"].as_str(), Some("info" | "action")));
        assert!(item.get("technical_details").is_some());
        for field in ["code", "summary", "detail", "remediation", "detected_at"] {
            assert!(
                item[field].is_string(),
                "legacy field {field} missing: {item}"
            );
        }
    }
    legacy.insert("active", false);
    db.collection::<Document>("startup_diagnostics")
        .replace_one(doc! {"_id":"legacy"}, legacy)
        .await
        .unwrap();
    db.collection::<Document>("users")
        .delete_one(doc! {"_id":organization})
        .await
        .unwrap();
    let result = active_startup_diagnostics(&db).await.unwrap();
    assert!(result.iter().all(|item| !matches!(
        item.code.as_str(),
        "exact_service_semantic_effect_index" | "organization_deletion_incomplete"
    )));
    db.drop().await.unwrap();
}

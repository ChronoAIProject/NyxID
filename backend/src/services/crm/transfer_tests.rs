use super::*;
use serde_json::json;
#[tokio::test]
async fn crm_import_exact_decimal_currency_and_sparse_mapping() {
    let db = crate::test_utils::connect_transaction_test_database("crm_currency_import").await;
    super::super::ensure_indexes(&db).await.unwrap();
    let owner = uuid::Uuid::new_v4().to_string();
    super::super::objects::ensure_seeded(&db, &owner)
        .await
        .unwrap();
    let object = super::super::objects::get(&db, &owner, "opportunity")
        .await
        .unwrap();
    let input = ImportInput {
        object: "opportunity".into(),
        mapping: BTreeMap::from([
            ("amount".into(), "amount".into()),
            ("name".into(), "name".into()),
        ]),
        rows: vec![],
        match_field: "name".into(),
        policy: MatchPolicy::CreateOnly,
        snapshot: None,
    };
    for (source, expected) in [
        ("90071992547409.91", 9_007_199_254_740_991_i64),
        ("-90071992547409.91", -9_007_199_254_740_991_i64),
        ("-1.01", -101),
        ("0.00", 0),
    ] {
        let row = BTreeMap::from([("amount".into(), json!(source))]);
        assert_eq!(
            convert(&object, &input, &row).unwrap()["amount"],
            json!(expected)
        );
    }
    for source in ["--1", "-+1", "+1", "90071992547409.92", "1.001", "NaN"] {
        assert!(
            convert(
                &object,
                &input,
                &BTreeMap::from([("amount".into(), json!(source))])
            )
            .is_err(),
            "{source}"
        );
    }
    assert!(
        convert(&object, &input, &BTreeMap::new())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        convert(
            &object,
            &input,
            &BTreeMap::from([("amount".into(), Value::Null)])
        )
        .unwrap()["amount"],
        Value::Null
    );
    db.drop().await.unwrap();
}

#[test]
fn crm_import_identity_comparison_uses_unicode_caseless_matching() {
    assert!(same_identity("Sigma Σ", "  sigma ς "));
    assert!(!same_identity("Sigma Σ", "Sigma T"));
}

#[tokio::test]
async fn crm_import_rejects_unicode_caseless_duplicate_identities_atomically() {
    let db = crate::test_utils::connect_transaction_test_database("crm_unicode_import").await;
    super::super::ensure_indexes(&db).await.unwrap();
    let owner = uuid::Uuid::new_v4().to_string();
    super::super::objects::ensure_seeded(&db, &owner)
        .await
        .unwrap();
    let mut input = ImportInput {
        object: "company".into(),
        mapping: BTreeMap::from([("name".into(), "name".into())]),
        rows: vec![
            BTreeMap::from([("name".into(), json!("Σ"))]),
            BTreeMap::from([("name".into(), json!("ς"))]),
        ],
        match_field: "name".into(),
        policy: MatchPolicy::UpdateOrCreate,
        snapshot: None,
    };
    let preview = import(
        &db,
        &owner,
        crate::models::crm_record::CrmActor::human(&owner),
        input.clone(),
        false,
        None,
    )
    .await
    .unwrap();
    assert_eq!(preview.rows[0].action, "create");
    assert_eq!(preview.rows[1].action, "error");
    assert_eq!(
        preview.rows[1].errors[0].message,
        "duplicate identity within import"
    );

    input.snapshot = Some(preview.snapshot);
    assert!(matches!(
        import(
            &db,
            &owner,
            crate::models::crm_record::CrmActor::human(&owner),
            input,
            true,
            None,
        )
        .await,
        Err(AppError::CrmValidationFailed(_))
    ));
    let records =
        db.collection::<mongodb::bson::Document>(crate::models::crm_record::COLLECTION_NAME);
    assert_eq!(
        records
            .count_documents(doc! {"owner_id": &owner, "object_key": "company"})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

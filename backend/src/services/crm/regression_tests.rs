//! Cross-service regressions exercise actual replica-set transactions and Mongo aggregation.
use super::*;
use crate::{
    errors::AppError,
    models::{
        crm_object::*,
        crm_record::{CrmActor, CrmActorKind, CrmRecord, CrmSource},
    },
};
use chrono::{Duration, Utc};
use mongodb::{
    Database,
    bson::{self, Document, doc},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

async fn setup(name: &str) -> (Database, String) {
    let db = crate::test_utils::connect_transaction_test_database(name).await;
    ensure_indexes(&db).await.unwrap();
    let owner = Uuid::new_v4().to_string();
    objects::ensure_seeded(&db, &owner).await.unwrap();
    (db, owner)
}
fn values(value: Value) -> BTreeMap<String, Value> {
    serde_json::from_value(value).unwrap()
}
async fn create(db: &Database, owner: &str, object: &str, value: Value) -> records::RecordResponse {
    records::create(
        db,
        owner,
        CrmActor::human(owner),
        records::CreateRecordInput {
            object: object.into(),
            values: values(value),
            protect: vec![],
        },
        None,
    )
    .await
    .unwrap()
}
async fn raw_rows(db: &Database, owner: &str, object: &str, rows: Vec<Document>) {
    let metadata = objects::get(db, owner, object).await.unwrap();
    let now = Utc::now();
    db.collection::<CrmRecord>(crate::models::crm_record::COLLECTION_NAME)
        .insert_many(rows.into_iter().map(|values| CrmRecord {
            id: Uuid::new_v4().to_string(),
            owner_id: owner.into(),
            object_id: metadata.id.clone(),
            object_key: object.into(),
            revision: 1,
            values,
            refs: Document::new(),
            overrides: vec![],
            field_sources: Document::new(),
            created_by: CrmActor::human(owner),
            updated_by: CrmActor::human(owner),
            archived_at: None,
            created_at: now,
            updated_at: now,
        }))
        .await
        .unwrap();
}
fn aggregate_query(
    object: &str,
    group: Option<&str>,
    measure: query::AggregateMeasure,
) -> query::CrmAggregateQuery {
    query::CrmAggregateQuery {
        object: object.into(),
        filters: vec![],
        group_by: group.map(str::to_owned),
        date_bucket: None,
        measure,
        order: query::AggregateOrder::LabelAsc,
    }
}

#[tokio::test]
async fn crm_import_snapshot_sparse_updates_replay_and_drift() {
    let (db, owner) = setup("crm_import_snapshot").await;
    let a = create(
        &db,
        &owner,
        "company",
        json!({"name":"A","domain":"a.test"}),
    )
    .await;
    let b = create(
        &db,
        &owner,
        "company",
        json!({"name":"B","domain":"b.test"}),
    )
    .await;
    let mut input = transfer::ImportInput {
        object: "company".into(),
        mapping: BTreeMap::from([
            ("name".into(), "name".into()),
            ("domain".into(), "domain".into()),
        ]),
        rows: vec![
            values(json!({"name":"A"})),
            values(json!({"name":"B","domain":null})),
        ],
        match_field: "name".into(),
        policy: transfer::MatchPolicy::UpdateOrCreate,
        snapshot: None,
    };
    let preview = transfer::import(
        &db,
        &owner,
        CrmActor::human(&owner),
        input.clone(),
        false,
        None,
    )
    .await
    .unwrap();
    assert_eq!(preview.rows[0].matched_id.as_deref(), Some(a.id.as_str()));
    input.snapshot = Some(preview.snapshot);
    let receipt = records::receipt_context(
        &owner,
        &owner,
        Some("import-sparse"),
        "POST",
        "/import/commit",
        &[],
        &input,
    )
    .unwrap();
    let committed = transfer::import(
        &db,
        &owner,
        CrmActor::human(&owner),
        input.clone(),
        true,
        receipt.clone(),
    )
    .await
    .unwrap();
    let replay = transfer::import(
        &db,
        &owner,
        CrmActor::human(&owner),
        input.clone(),
        true,
        receipt,
    )
    .await
    .unwrap();
    assert_eq!(json!(committed), json!(replay));
    assert_eq!(
        records::get(&db, &owner, &a.id).await.unwrap().values["domain"],
        json!("a.test")
    );
    assert!(
        records::get(&db, &owner, &b.id)
            .await
            .unwrap()
            .values
            .get("domain")
            .is_none_or(Value::is_null)
    );
    // A same-revision replacement must not redirect an inspected update.
    let mut drift = input.clone();
    drift.snapshot = None;
    let preview = transfer::import(
        &db,
        &owner,
        CrmActor::human(&owner),
        drift.clone(),
        false,
        None,
    )
    .await
    .unwrap();
    let inspected_revision = preview.rows[0].revision.unwrap();
    drift.snapshot = Some(preview.snapshot);
    db.collection::<Document>(crate::models::crm_record::COLLECTION_NAME)
        .update_one(doc! {"_id":&a.id}, doc! {"$set":{"values.name":"Renamed"}})
        .await
        .unwrap();
    let replacement = create(&db, &owner, "company", json!({"name":"A"})).await;
    db.collection::<Document>(crate::models::crm_record::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&replacement.id},
            doc! {"$set":{"revision":inspected_revision}},
        )
        .await
        .unwrap();
    assert_eq!(
        records::get(&db, &owner, &replacement.id)
            .await
            .unwrap()
            .revision,
        inspected_revision
    );
    assert!(matches!(
        transfer::import(&db, &owner, CrmActor::human(&owner), drift, true, None).await,
        Err(AppError::CrmValidationFailed(_))
    ));
    assert!(
        !records::get(&db, &owner, &replacement.id)
            .await
            .unwrap()
            .values
            .contains_key("domain")
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_import_schema_and_revision_drift_are_atomic() {
    let (db, owner) = setup("crm_import_drift").await;
    let record = create(
        &db,
        &owner,
        "company",
        json!({"name":"Existing","domain":"original.test"}),
    )
    .await;
    let mut input = transfer::ImportInput {
        object: "company".into(),
        mapping: BTreeMap::from([
            ("name".into(), "name".into()),
            ("domain".into(), "domain".into()),
        ]),
        rows: vec![
            values(json!({"name":"New","domain":"new.test"})),
            values(json!({"name":"Existing","domain":"import.test"})),
        ],
        match_field: "name".into(),
        policy: transfer::MatchPolicy::UpdateOrCreate,
        snapshot: None,
    };
    input.snapshot = Some(
        transfer::import(
            &db,
            &owner,
            CrmActor::human(&owner),
            input.clone(),
            false,
            None,
        )
        .await
        .unwrap()
        .snapshot,
    );
    records::patch(
        &db,
        &owner,
        &record.id,
        CrmActor::human(&owner),
        records::PatchRecordInput {
            expected_revision: 1,
            values: values(json!({"domain":"human.test"})),
            protect: vec![],
            unprotect: vec![],
        },
        None,
    )
    .await
    .unwrap();
    assert!(
        transfer::import(
            &db,
            &owner,
            CrmActor::human(&owner),
            input.clone(),
            true,
            None
        )
        .await
        .is_err()
    );
    assert_eq!(
        db.collection::<Document>(crate::models::crm_record::COLLECTION_NAME)
            .count_documents(doc! {"owner_id":&owner})
            .await
            .unwrap(),
        1
    );
    input.snapshot = None;
    input.snapshot = Some(
        transfer::import(
            &db,
            &owner,
            CrmActor::human(&owner),
            input.clone(),
            false,
            None,
        )
        .await
        .unwrap()
        .snapshot,
    );
    objects::patch_field(
        &db,
        &owner,
        "company",
        "domain",
        objects::PatchField {
            expected_schema_revision: 1,
            label: None,
            required: None,
            archived: Some(true),
            options: None,
        },
        None,
    )
    .await
    .unwrap();
    assert!(
        transfer::import(&db, &owner, CrmActor::human(&owner), input, true, None)
            .await
            .is_err()
    );
    assert_eq!(
        records::get(&db, &owner, &record.id).await.unwrap().values["domain"],
        json!("human.test")
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_source_order_protection_and_patch_stage_history() {
    let (db, owner) = setup("crm_source_order").await;
    let record = create(
        &db,
        &owner,
        "opportunity",
        json!({"name":"Deal","stage":"new","amount":100}),
    )
    .await;
    let now = Utc::now() + Duration::minutes(1);
    let agent = |time| CrmActor {
        kind: CrmActorKind::Agent,
        user_id: owner.clone(),
        agent_id: Some(Uuid::new_v4().to_string()),
        api_key_id: None,
        source: Some(CrmSource {
            conversation_id: "conversation".into(),
            turn_id: "turn".into(),
            run_id: Some("run".into()),
            ordered_at: time,
        }),
    };
    let patch = |revision, value| records::PatchRecordInput {
        expected_revision: revision,
        values: values(value),
        protect: vec![],
        unprotect: vec![],
    };
    let fresh = records::patch(
        &db,
        &owner,
        &record.id,
        agent(now),
        patch(1, json!({"amount":200,"stage":"qualified"})),
        None,
    )
    .await
    .unwrap();
    let stale = records::patch(
        &db,
        &owner,
        &record.id,
        agent(now - Duration::seconds(1)),
        patch(fresh.revision, json!({"amount":1,"stage":"new"})),
        None,
    )
    .await
    .unwrap();
    assert_eq!(stale.values["amount"]["amount_minor"], json!(200));
    assert_eq!(stale.skipped_stale.len(), 2);
    let protected = records::patch(
        &db,
        &owner,
        &record.id,
        CrmActor::human(&owner),
        records::PatchRecordInput {
            protect: vec!["amount".into()],
            ..patch(stale.revision, json!({}))
        },
        None,
    )
    .await
    .unwrap();
    let skipped = records::patch(
        &db,
        &owner,
        &record.id,
        agent(now + Duration::seconds(1)),
        patch(protected.revision, json!({"amount":300})),
        None,
    )
    .await
    .unwrap();
    assert_eq!(skipped.skipped_protected, vec!["amount"]);
    let event = db
        .collection::<Document>(crate::models::crm_activity::COLLECTION_NAME)
        .find_one(doc! {"record_id":&record.id,"kind":"stage_changed"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        event
            .get_document("summary")
            .unwrap()
            .get_str("from")
            .unwrap(),
        "new"
    );
    assert_eq!(
        event
            .get_document("summary")
            .unwrap()
            .get_str("to")
            .unwrap(),
        "qualified"
    );
    let encoded = serde_json::to_value(
        activities::list(&db, &owner, &record.id, None)
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(!encoded.to_string().contains("$date"));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_patch_activity_is_one_event_with_all_changed_fields() {
    use crate::models::crm_activity::CrmActivityKind;

    let (db, owner) = setup("crm_patch_activity").await;
    let mut record = create(
        &db,
        &owner,
        "opportunity",
        json!({"name":"Deal","stage":"new","amount":100}),
    )
    .await;
    for (values, kind, fields, from, to) in [
        (
            json!({"stage":"qualified"}),
            CrmActivityKind::StageChanged,
            vec!["stage"],
            Some("new"),
            Some("qualified"),
        ),
        (
            json!({"stage":"proposal","amount":200}),
            CrmActivityKind::StageChanged,
            vec!["amount", "stage"],
            Some("qualified"),
            Some("proposal"),
        ),
        (
            json!({"amount":300}),
            CrmActivityKind::Updated,
            vec!["amount"],
            None,
            None,
        ),
    ] {
        let before = activities::list(&db, &owner, &record.id, None)
            .await
            .unwrap();
        record = records::patch(
            &db,
            &owner,
            &record.id,
            CrmActor::human(&owner),
            records::PatchRecordInput {
                expected_revision: record.revision,
                values: serde_json::from_value(values).unwrap(),
                protect: vec![],
                unprotect: vec![],
            },
            None,
        )
        .await
        .unwrap();
        let after = activities::list(&db, &owner, &record.id, None)
            .await
            .unwrap();
        let added: Vec<_> = after
            .activities
            .iter()
            .filter(|event| !before.activities.iter().any(|old| old.id == event.id))
            .collect();
        assert_eq!(added.len(), 1, "one history row per logical patch");
        let event = added[0];
        assert_eq!(event.kind, kind);
        assert_eq!(
            event.summary.changed_fields.as_ref().unwrap(),
            &fields.into_iter().map(str::to_owned).collect::<Vec<_>>()
        );
        assert_eq!(event.summary.from.as_deref(), from);
        assert_eq!(event.summary.to.as_deref(), to);
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_aggregates_preserve_unknown_other_zero_and_reject_overflow() {
    let (db, owner) = setup("crm_aggregate_unknown").await;
    let options = (0..55)
        .map(|n| CrmSelectOption {
            value: format!("g{n}"),
            label: format!("Group {n:02}"),
            archived: false,
            color: None,
        })
        .collect();
    objects::add_field(
        &db,
        &owner,
        "company",
        1,
        CrmField {
            key: "category".into(),
            label: "Category".into(),
            required: false,
            archived: false,
            kind: CrmFieldKind::Select { options },
        },
        None,
    )
    .await
    .unwrap();
    objects::add_field(
        &db,
        &owner,
        "company",
        2,
        CrmField {
            key: "score".into(),
            label: "Score".into(),
            required: false,
            archived: false,
            kind: CrmFieldKind::Number {
                format: CrmNumberFormat::Plain,
            },
        },
        None,
    )
    .await
    .unwrap();
    let query = aggregate_query(
        "company",
        Some("category"),
        query::AggregateMeasure::Sum {
            field: "score".into(),
        },
    );
    let empty = query::aggregate(&db, &owner, &query).await.unwrap();
    assert_eq!(empty.total, Value::Null);
    assert!(empty.points.is_empty());
    let count = query::aggregate(
        &db,
        &owner,
        &aggregate_query("company", None, query::AggregateMeasure::Count),
    )
    .await
    .unwrap();
    assert_eq!(count.total, json!(0));
    raw_rows(
        &db,
        &owner,
        "company",
        (0..55)
            .map(|n| doc! {"name":format!("Company {n}"),"category":format!("g{n}")})
            .collect(),
    )
    .await;
    let unknown = query::aggregate(&db, &owner, &query).await.unwrap();
    assert!(unknown.truncated);
    assert_eq!(unknown.total, Value::Null);
    assert!(unknown.points.iter().all(|p| p.value.is_null()));
    assert_eq!(unknown.points.last().unwrap().label, "Other");
    db.collection::<Document>(crate::models::crm_record::COLLECTION_NAME)
        .update_one(
            doc! {"owner_id":&owner,"values.category":"g54"},
            doc! {"$set":{"values.score":0}},
        )
        .await
        .unwrap();
    let zero = query::aggregate(&db, &owner, &query).await.unwrap();
    assert_eq!(zero.total.as_f64(), Some(0.0));
    assert_eq!(zero.points.last().unwrap().value.as_f64(), Some(0.0));
    db.collection::<Document>(crate::models::crm_record::COLLECTION_NAME)
        .update_many(
            doc! {"owner_id":&owner},
            doc! {"$set":{"values.score":1.7e308_f64}},
        )
        .await
        .unwrap();
    assert!(matches!(
        query::aggregate(&db, &owner, &query).await,
        Err(AppError::CrmValidationFailed(_))
    ));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_date_buckets_use_explicit_timezone_and_calendar_dates_do_not_shift() {
    let (db, owner) = setup("crm_date_buckets").await;
    for (revision, key, kind) in [
        (1, "observed", CrmFieldKind::Datetime {}),
        (2, "calendar", CrmFieldKind::Date {}),
    ] {
        objects::add_field(
            &db,
            &owner,
            "company",
            revision,
            CrmField {
                key: key.into(),
                label: key.into(),
                required: false,
                archived: false,
                kind,
            },
            None,
        )
        .await
        .unwrap();
    }
    raw_rows(
        &db,
        &owner,
        "company",
        vec![
            doc! {"name":"A","observed":"2026-11-03T16:30:00Z","calendar":"2026-11-03"},
            doc! {"name":"B","observed":"2026-11-03T15:30:00Z","calendar":"2026-11-03"},
        ],
    )
    .await;
    let mut query = aggregate_query("company", Some("observed"), query::AggregateMeasure::Count);
    query.date_bucket = Some(query::DateBucket {
        unit: query::CalendarUnit::Day,
        timezone: "Asia/Singapore".into(),
    });
    let result = query::aggregate(&db, &owner, &query).await.unwrap();
    assert_eq!(
        result
            .points
            .iter()
            .map(|p| p.label.as_str())
            .collect::<Vec<_>>(),
        vec!["2026-11-03", "2026-11-04"]
    );
    query.group_by = Some("calendar".into());
    assert!(query::aggregate(&db, &owner, &query).await.is_err());
    query.date_bucket.as_mut().unwrap().timezone = "UTC".into();
    let result = query::aggregate(&db, &owner, &query).await.unwrap();
    assert_eq!(result.points.len(), 1);
    assert_eq!(result.points[0].label, "2026-11-03");
    assert_eq!(result.total, json!(2));
    db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_user_group_labels_match_owner_scoped_people_eligibility() {
    let (db, owner) = setup("crm_people_aggregate").await;
    let ids: Vec<_> = (0..4).map(|_| Uuid::new_v4().to_string()).collect();
    for (index, id) in ids.iter().enumerate() {
        let mut user = crate::test_utils::test_user(id, crate::models::user::UserType::Person);
        user.display_name = Some(format!("Private name {index}"));
        user.is_active = index != 2;
        db.collection(crate::models::user::COLLECTION_NAME)
            .insert_one(user)
            .await
            .unwrap();
    }
    for (index, id) in ids.iter().take(3).enumerate() {
        db.collection::<Document>(crate::models::org_membership::COLLECTION_NAME).insert_one(doc!{"_id":Uuid::new_v4().to_string(),"org_user_id":&owner,"member_user_id":id,"revoked_at":if index==1 {bson::Bson::DateTime(bson::DateTime::now())} else {bson::Bson::Null}}).await.unwrap();
    }
    raw_rows(
        &db,
        &owner,
        "company",
        ids.iter()
            .enumerate()
            .map(|(n, id)| doc! {"name":format!("Company {n}"),"owner":id})
            .collect(),
    )
    .await;
    let result = query::aggregate(
        &db,
        &owner,
        &aggregate_query("company", Some("owner"), query::AggregateMeasure::Count),
    )
    .await
    .unwrap();
    let people = people::lookup(&db, &owner, &ids).await.unwrap();
    for person in people {
        let point = result
            .points
            .iter()
            .find(|p| p.key == json!(person.id))
            .unwrap();
        assert_eq!(point.label, person.label);
    }
    assert_eq!(
        result
            .points
            .iter()
            .filter(|p| p.label == "Unavailable member")
            .count(),
        3
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn crm_record_write_waits_for_owner_schema_fence_and_revalidates_after_commit() {
    let (db, owner) = setup("crm_schema_fence_race").await;
    let mut tx = db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    super::fence_owner(&db, &owner, &mut tx).await.unwrap();
    db.collection::<Document>(crate::models::crm_object::COLLECTION_NAME)
        .update_one(
            doc! {"owner_id":&owner,"key":"company","fields.key":"domain"},
            doc! {"$set":{"fields.$.archived":true},"$inc":{"schema_revision":1_i64}},
        )
        .session(&mut tx)
        .await
        .unwrap();
    let pending = {
        let db = db.clone();
        let owner = owner.clone();
        tokio::spawn(async move {
            records::create(
                &db,
                &owner,
                CrmActor::human(&owner),
                records::CreateRecordInput {
                    object: "company".into(),
                    values: values(json!({"name":"Company","domain":"stale.test"})),
                    protect: vec![],
                },
                None,
            )
            .await
        })
    };
    tokio::task::yield_now().await;
    tx.commit_transaction().await.unwrap();
    assert!(matches!(
        pending.await.unwrap(),
        Err(AppError::CrmValidationFailed(_))
    ));
    assert_eq!(
        db.collection::<Document>(crate::models::crm_record::COLLECTION_NAME)
            .count_documents(doc! {"owner_id":&owner})
            .await
            .unwrap(),
        0
    );
    db.drop().await.unwrap();
}

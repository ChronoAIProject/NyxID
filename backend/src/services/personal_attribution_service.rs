//! One redaction contract for durable erasure and the pre-job orphan sweep.
use bson::{Bson, Document, doc};

use super::personal_data_registry::Attribution;

pub const DELETED_ACCOUNT: &str = "Deleted account";

pub fn pending_filter(spec: &Attribution, person: Option<&str>) -> Document {
    let mut filter = doc! {};
    filter.insert(
        spec.person_field,
        person
            .map(Bson::from)
            .unwrap_or_else(|| doc! {"$exists": true}.into()),
    );
    let fields: Vec<Document> = spec
        .name_fields
        .iter()
        .map(|field| {
            let mut condition = Document::new();
            condition.insert(
                *field,
                doc! {"$exists": true, "$nin": [DELETED_ACCOUNT, Bson::Null]},
            );
            condition
        })
        .chain(spec.email_fields.iter().map(|field| {
            let mut condition = Document::new();
            condition.insert(*field, doc! {"$exists": true});
            condition
        }))
        .collect();
    filter.insert("$or", fields);
    filter
}

/// Only existing name fields are replaced; absent fields remain absent. Email
/// snapshots are removed. Every opaque id and all resource content is preserved.
pub fn update(spec: &Attribution) -> Vec<Document> {
    let mut fields = Document::new();
    for field in spec.name_fields {
        fields.insert(
            *field,
            doc! {"$cond": [
                {"$in": [{"$type": format!("${field}")}, ["missing", "null"]]},
                format!("${field}"), DELETED_ACCOUNT,
            ]},
        );
    }
    for field in spec.email_fields {
        fields.insert(*field, "$$REMOVE");
    }
    vec![doc! {"$set": fields}]
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use futures::TryStreamExt;

    pub const NAME: &str = "Attribution Alice Sentinel";
    pub const EMAIL: &str = "attribution-alice-sentinel@example.test";

    pub struct Case {
        pub collection: &'static str,
        pub id: String,
        pub before: Document,
        pub after: Document,
        pub bystander: Document,
    }

    pub fn put(row: &mut Document, path: &str, value: Bson) {
        if let Some((head, tail)) = path.split_once('.') {
            if !row.contains_key(head) {
                row.insert(head, Document::new());
            }
            put(row.get_document_mut(head).unwrap(), tail, value);
        } else {
            row.insert(path, value);
        }
    }

    fn remove(row: &mut Document, path: &str) {
        if let Some((head, tail)) = path.split_once('.') {
            if let Ok(child) = row.get_document_mut(head) {
                remove(child, tail);
            }
        } else {
            row.remove(path);
        }
    }

    /// Seed every reviewed persisted actor shape, with more than one worker
    /// page, mixed attribution, an already-replaced name with leftover email,
    /// and missing legacy fields that must not be created by redaction.
    pub async fn seed(
        db: &mongodb::Database,
        person: &str,
        resource_owner: &str,
        count: usize,
    ) -> Vec<Case> {
        let mut cases = Vec::new();
        for (index, spec) in super::super::personal_data_registry::ATTRIBUTIONS
            .iter()
            .enumerate()
        {
            for page in 0..count {
                let id = format!("attribution-{index}-{page}");
                let mut before = doc! {
                    "_id": &id, "user_id": resource_owner, "owner_id": resource_owner,
                    "group_id": "shared-group", "counter": 47,
                    "created_by": {"actor": {"person_id": resource_owner, "name": "Other creator"}},
                    "last_change": {"actor": {"person_id": resource_owner, "name": "Other updater"}},
                };
                put(&mut before, spec.person_field, person.into());
                if let Some((actor, _)) = spec.person_field.rsplit_once('.') {
                    for (field, value) in [
                        ("id", "opaque-key-id"),
                        ("kind", "api_key"),
                        ("api_key_id", "opaque-key-id"),
                        ("app_id", "opaque-app-id"),
                    ] {
                        put(&mut before, &format!("{actor}.{field}"), value.into());
                    }
                }
                for field in spec.name_fields {
                    put(
                        &mut before,
                        field,
                        if page == 0 && !spec.email_fields.is_empty() {
                            DELETED_ACCOUNT.into()
                        } else {
                            NAME.into()
                        },
                    );
                }
                for field in spec.email_fields {
                    put(&mut before, field, EMAIL.into());
                }
                if page == 1 && spec.name_fields.len() > 1 {
                    remove(&mut before, spec.name_fields[1]);
                } else if page == 2 && spec.name_fields.len() > 1 {
                    put(&mut before, spec.name_fields[1], Bson::Null);
                }
                let mut after = before.clone();
                for field in spec.name_fields {
                    if !(matches!(page, 1 | 2) && spec.name_fields.get(1) == Some(field)) {
                        put(&mut after, field, DELETED_ACCOUNT.into());
                    }
                }
                for field in spec.email_fields {
                    remove(&mut after, field);
                }
                let mut bystander = before.clone();
                bystander.insert("_id", format!("{id}-bystander"));
                put(&mut bystander, spec.person_field, resource_owner.into());
                for field in spec.name_fields {
                    put(&mut bystander, field, "Other actor".into());
                }
                for field in spec.email_fields {
                    put(&mut bystander, field, "other@example.test".into());
                }
                db.collection::<Document>(spec.collection)
                    .insert_one(before.clone())
                    .await
                    .unwrap();
                db.collection::<Document>(spec.collection)
                    .insert_one(bystander.clone())
                    .await
                    .unwrap();
                cases.push(Case {
                    collection: spec.collection,
                    id,
                    before,
                    after,
                    bystander,
                });
            }
        }
        cases
    }

    pub async fn assert_no_identity_outside_audit(db: &mongodb::Database) {
        for collection in db.list_collection_names().await.unwrap() {
            if collection == crate::models::audit_log::COLLECTION_NAME {
                continue;
            }
            let mut cursor = db
                .collection::<Document>(&collection)
                .find(doc! {})
                .await
                .unwrap();
            while let Some(row) = cursor.try_next().await.unwrap() {
                let serialized = serde_json::to_string(&row).unwrap();
                assert!(
                    !serialized.contains(NAME),
                    "name survived in {collection}: {row:?}"
                );
                assert!(
                    !serialized.contains(EMAIL),
                    "email survived in {collection}: {row:?}"
                );
            }
        }
    }

    pub async fn assert_cases(db: &mongodb::Database, cases: &[Case], deleted_messages: bool) {
        for case in cases {
            assert_eq!(
                db.collection::<Document>(case.collection)
                    .find_one(doc! {"_id": case.bystander.get("_id").unwrap()})
                    .await
                    .unwrap(),
                Some(case.bystander.clone()),
                "bystander changed in {}",
                case.collection
            );
            let actual = db
                .collection::<Document>(case.collection)
                .find_one(doc! {"_id": &case.id})
                .await
                .unwrap();
            if deleted_messages && case.collection == "assistant_group_messages" && actual.is_none()
            {
                continue;
            }
            assert_eq!(
                actual,
                Some(case.after.clone()),
                "redaction drift in {}:{}",
                case.collection,
                case.id
            );
        }
    }
}

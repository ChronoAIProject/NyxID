//! Storage CAS preserves the exact BSON source representation. Model decoding
//! deliberately accepts historical byte arrays, whereas serialization emits
//! Binary; a model roundtrip cannot supply an exact-source migration predicate.
use bson::{Bson, Document, doc};

pub(crate) fn source_predicate(source: &Document, fields: &[&str]) -> Document {
    let comparisons: Vec<Bson> = fields
        .iter()
        .map(|field| {
            let path = format!("${field}");
            match source.get(*field) {
                Some(value) => bson::bson!({"$and":[
                    {"$ne":[{"$type":&path},"missing"]},
                    {"$eq":[&path,{"$literal":value}]}
                ]}),
                None => bson::bson!({"$eq":[{"$type":&path},"missing"]}),
            }
        })
        .collect();
    // IDs stay outside $expr to preserve the primary-key point lookup. Every
    // other comparison retains both missing-vs-null and array-vs-Binary shape.
    doc! {"_id": source.get("_id").cloned().unwrap_or(Bson::Null), "$expr":{"$and":comparisons}}
}
/// Exact raw-document CAS also fences formerly absent version/relationship
/// fields. Comparing only present keys cannot protect a replaced version map.
pub(crate) fn exact_document_predicate(source: &bson::Document) -> bson::Document {
    // MongoDB moves _id to the first position when inserting a document.
    // Model serialization need not put it first. Preserve every other raw
    // field/value and its order while comparing the server's stored layout.
    let id = source.get("_id").cloned().unwrap_or(bson::Bson::Null);
    let mut stored = bson::doc! {"_id":id.clone()};
    stored.extend(
        source
            .iter()
            .filter(|(key, _)| key.as_str() != "_id")
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    bson::doc! {"_id":id,"$expr":{"$eq":["$$ROOT",{"$literal":stored}]}}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn raw_cas_accepts_server_id_reordering_but_rejects_new_fields_and_byte_shapes() {
        let db = crate::test_utils::connect_transaction_test_database("raw_cas_id_order").await;
        let rows = db.collection::<Document>("raw_cas_fixtures");
        let id = uuid::Uuid::new_v4().to_string();
        let source = doc! {"version":1_i32,"_id":&id,"bytes":[1_i32,2_i32],"optional":Bson::Null};
        rows.insert_one(&source).await.unwrap();
        let stored = rows.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
        assert_eq!(stored.keys().next().map(String::as_str), Some("_id"));
        assert!(
            rows.find_one(exact_document_predicate(&source))
                .await
                .unwrap()
                .is_some()
        );
        rows.update_one(doc! {"_id":&id}, doc! {"$set":{"new_field":"must survive"}})
            .await
            .unwrap();
        assert!(
            rows.find_one(exact_document_predicate(&source))
                .await
                .unwrap()
                .is_none()
        );
        rows.update_one(doc! {"_id":&id}, doc! {"$unset":{"new_field":""},"$set":{"bytes":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![1,2]}}}).await.unwrap();
        assert!(
            rows.find_one(exact_document_predicate(&source))
                .await
                .unwrap()
                .is_none()
        );
        db.drop().await.unwrap();
    }
}

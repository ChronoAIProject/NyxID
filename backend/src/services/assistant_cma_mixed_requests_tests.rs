use super::*;
use crate::models::assistant_cma_sandbox::COLLECTION_NAME as PARENTS;
use crate::services::encrypted_record_migration as migration;
use bson::Document;

const BODY: &[u8] =
    r#"{ "name":"first", "name":"café\u0021", "machine_included":false }"#.as_bytes();
const KEY: &str = "mixed-cma-request-key";

#[tokio::test]
async fn mixed_cma_request_copies() {
    let step = std::env::var("NYXID_RECORD_FIXTURE_STEP").unwrap_or_else(|_| "roundtrip".into());
    let file = std::env::var_os("NYXID_RECORD_FIXTURE_FILE");
    let mut f = Fixture::new().await;
    let writes = std::env::var("CONTEXTUAL_RECORD_WRITES").as_deref() == Ok("true");
    f.keys.set_contextual_record_writes(writes);
    f.keys.set_allow_legacy_record_reads(
        std::env::var("ALLOW_LEGACY_RECORD_READS").as_deref() != Ok("false"),
    );
    let collection = f.db.collection::<Document>(COLLECTION_NAME);
    let parents = f.db.collection::<Document>(PARENTS);
    let id = if matches!(step.as_str(), "roundtrip" | "write") {
        let permit = f
            .admit(&f.target, "/api/v1/sandboxes", KEY, BODY)
            .await
            .unwrap();
        permit.creation.unwrap().id
    } else {
        let fixture: Document =
            bson::from_slice(&std::fs::read(file.as_ref().unwrap()).unwrap()).unwrap();
        let creation = fixture.get_document("creation").unwrap();
        collection.insert_one(creation).await.unwrap();
        parents
            .insert_one(fixture.get_document("sandbox").unwrap())
            .await
            .unwrap();
        f.owner = creation.get_str("user_id").unwrap().to_owned();
        f.db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(&f.owner, UserType::Person))
            .await
            .unwrap();
        let catalogs = f
            .db
            .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME);
        catalogs
            .delete_one(doc! {"_id":&f.catalog.id})
            .await
            .unwrap();
        f.catalog = bson::from_document(fixture.get_document("catalog").unwrap().clone()).unwrap();
        catalogs.insert_one(&f.catalog).await.unwrap();
        f.target = Fixture::connection_target(f.catalog.clone());
        creation.get_str("_id").unwrap().to_owned()
    };
    let original = f.saved(&id).await;
    match step.as_str() {
        "clear" | "replace" => {
            if original.submission_state != "confirmed_not_created" {
                change_creation(&f.db, &original, CreationChange::NoResource)
                    .await
                    .unwrap();
            }
            if step == "replace" {
                let permit = f
                    .admit(&f.target, "/api/v1/sandboxes", KEY, BODY)
                    .await
                    .unwrap();
                let next = permit.creation.unwrap();
                assert_eq!(next.id, id);
                assert_eq!(next.sandbox_ref_id, original.sandbox_ref_id);
                assert_eq!(
                    next.submission_generation,
                    original.submission_generation + 1
                );
                assert!(next.sandbox_generation > original.sandbox_generation);
            }
        }
        "migrate" => {
            let generation = migration::start(&f.db, &f.keys, migration::Mode::Migrate, None)
                .await
                .unwrap();
            loop {
                let result = migration::status(&f.db, &f.keys).await.unwrap().unwrap();
                if result.phase == "observed" {
                    for name in [PARENTS, COLLECTION_NAME] {
                        let field = result
                            .field_outcomes
                            .iter()
                            .find(|v| v.collection == name)
                            .unwrap();
                        assert_eq!(field.authenticated_current, 1);
                        assert_eq!(field.verification_blocked, 0);
                    }
                    assert_eq!(result.rewritten, 2);
                    break;
                }
                migration::advance(&f.db, &f.keys, &generation)
                    .await
                    .unwrap();
            }
        }
        "roundtrip" | "write" | "read" | "reject" => (),
        _ => panic!("unknown CMA fixture step"),
    }
    let row = f.saved(&id).await;
    let sandbox = sandboxes::get_owned(&f.db, &f.owner, &row.sandbox_ref_id)
        .await
        .unwrap();
    assert_eq!(row.idempotency_key, KEY);
    assert_eq!(sandbox.generation, row.sandbox_generation);
    assert_eq!(
        sandbox.request_hash.as_deref(),
        Some(row.request_hash.as_str())
    );
    if row.submission_state == "confirmed_not_created" {
        assert!(!sandbox.occupies_account_slot);
        assert!(row.request_ciphertext.is_none() && sandbox.request_ciphertext.is_none());
        assert!(row.request_context_version.is_none() && sandbox.request_context_version.is_none());
    } else {
        if matches!(step.as_str(), "roundtrip" | "write" | "replace" | "migrate") {
            assert_eq!(row.request_context_version, writes.then_some(3));
        }
        let context =
            RecordContext::new(COLLECTION_NAME, &id, &f.owner, &["request_ciphertext"]).unwrap();
        let version = row
            .request_context_version
            .map(|v| u8::try_from(v).unwrap());
        let creation = crate::crypto::record_field::decrypt_context(
            &f.keys,
            &context,
            version,
            row.request_ciphertext.as_deref().unwrap(),
        )
        .await;
        let parent = sandboxes::recover_creation_request(
            &f.db,
            &f.keys,
            &f.owner,
            &sandbox.id,
            sandbox.generation,
        )
        .await;
        assert_eq!(row.request_context_version, sandbox.request_context_version);
        if step == "reject" {
            assert!(creation.is_err() && parent.is_err());
        } else {
            assert_eq!(creation.unwrap().as_slice(), BODY);
            assert_eq!(parent.unwrap().as_slice(), BODY);
            assert_eq!(
                sandboxes::prepare_creation_request(BODY).unwrap().hash,
                row.request_hash
            );
        }
        f.assert_single_intent().await;
    }
    if let Some(file) = file {
        let creation = collection
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap();
        let parent = parents
            .find_one(doc! {"_id":&sandbox.id})
            .await
            .unwrap()
            .unwrap();
        if row.request_ciphertext.is_some() {
            for source in [&creation, &parent] {
                assert!(source.get_binary_generic("request_ciphertext").is_ok());
                assert_eq!(
                    source.get("request_context_version"),
                    Some(&bson::to_bson(&row.request_context_version).unwrap())
                );
            }
        }
        let fixture = doc! {"creation":creation, "sandbox":parent, "catalog":bson::to_document(&f.catalog).unwrap()};
        std::fs::write(file, bson::to_vec(&fixture).unwrap()).unwrap();
    }
    assert_eq!(f.post_count().await, 0);
    f.db.drop().await.unwrap();
}

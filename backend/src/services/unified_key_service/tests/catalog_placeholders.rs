use super::*;
use crate::crypto::aes::EncryptionKeys;
use crate::handlers::services_helpers::service_to_response_with_viewer;
use crate::services::{provider_service, ssh_service};
use bson::Document;

async fn write_rows(keys: &EncryptionKeys, normalized: bool) -> Document {
    let db =
        crate::test_utils::connect_transaction_test_database("catalog_placeholder_writers").await;
    provider_service::seed_default_providers(&db, keys)
        .await
        .unwrap();
    provider_service::seed_default_services(&db, keys)
        .await
        .unwrap();
    let services =
        db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME);
    let lookup = doc! {"slug": "llm-openai"};
    let seeded = services.find_one(lookup.clone()).await.unwrap().unwrap();
    provider_service::seed_default_services(&db, keys)
        .await
        .unwrap();
    let reseeded = services.find_one(lookup).await.unwrap().unwrap();
    assert_eq!(reseeded.id, seeded.id);
    assert_eq!(reseeded.credential_encrypted, seeded.credential_encrypted);

    let owner = uuid::Uuid::new_v4().to_string();
    let node_id = uuid::Uuid::new_v4().to_string();
    insert_active_node(&db, &owner, &node_id).await;
    let created = create_key(
        &db,
        keys,
        &owner,
        &owner,
        None,
        None,
        "",
        "SSH fixture",
        None,
        None,
        None,
        Some(&node_id),
        Some(SshCreateParams {
            host: "ssh.example.test",
            port: 22,
            certificate_auth: true,
            ssh_auth_mode: crate::models::ssh_auth_mode::SshAuthMode::Cert,
            principals: vec!["fixture".into()],
            certificate_ttl_minutes: 60,
        }),
        None,
        OpenApiSpecUrlInput::Inherit,
        None,
        false,
        OauthClientCredentialsInput::None,
        false,
    )
    .await
    .unwrap();
    let ssh = services
        .find_one(doc! {"_id":created.service.catalog_service_id.unwrap()})
        .await
        .unwrap()
        .unwrap();
    let mut fixture = Document::new();
    for (name, row) in [("seed", seeded), ("ssh", ssh)] {
        assert_eq!(row.credential_encrypted.is_empty(), normalized);
        assert!(
            !row.secret_context_versions
                .contains_key("credential_encrypted")
        );
        if !normalized {
            assert_eq!(row.credential_encrypted.first(), Some(&2));
        }
        let response = service_to_response_with_viewer(Some(keys), row.clone(), None).await;
        assert_eq!(response.credential_configured, Some(false));
        let mut empty = row.clone();
        empty.credential_encrypted.clear();
        let normalized_view = service_to_response_with_viewer(Some(keys), empty, None).await;
        assert_eq!(
            bson::to_document(&response).unwrap(),
            bson::to_document(&normalized_view).unwrap()
        );
        fixture.insert(name, bson::to_document(&row).unwrap());
        fixture.insert(
            format!("{name}_response"),
            bson::to_document(&response).unwrap(),
        );
    }
    fixture
}

async fn read_rows(keys: &EncryptionKeys, fixture: &Document) {
    for name in ["seed", "ssh"] {
        let row: DownstreamService =
            bson::from_document(fixture.get_document(name).unwrap().clone()).unwrap();
        let response = service_to_response_with_viewer(Some(keys), row.clone(), None).await;
        let mut expected = fixture
            .get_document(format!("{name}_response"))
            .unwrap()
            .clone();
        if !row.credential_encrypted.is_empty() && !keys.allow_legacy_record_reads() {
            assert_eq!(response.credential_configured, None);
            expected.insert("credential_configured", bson::Bson::Null);
        } else {
            assert_eq!(response.credential_configured, Some(false));
        }
        assert_eq!(bson::to_document(&response).unwrap(), expected);
        let ordinary = service_to_response_with_viewer(None, row.clone(), None).await;
        assert_eq!(ordinary.credential_configured, None);
        if let Some(ssh) = &row.ssh_config {
            let cert = ssh_service::issue_certificate(
                keys,
                ssh,
                &row.id,
                &row.created_by,
                "fixture@example.test",
                ssh.ca_public_key.as_deref().unwrap(),
                "fixture",
                false,
            )
            .await;
            if ssh.ca_context_version.is_some() || keys.allow_legacy_record_reads() {
                assert_eq!(cert.unwrap().principal, "fixture");
            } else {
                assert!(cert.is_err());
            }
        }
    }
}

#[tokio::test]
async fn catalog_placeholder_management_and_ssh_roundtrip() {
    let mut keys = test_encryption_keys();
    if let Ok(step) = std::env::var("NYXID_RECORD_FIXTURE_STEP") {
        let file = std::env::var("NYXID_RECORD_FIXTURE_FILE").unwrap();
        keys.set_contextual_record_writes(
            std::env::var("CONTEXTUAL_RECORD_WRITES").unwrap() == "true",
        );
        keys.set_allow_legacy_record_reads(
            std::env::var("ALLOW_LEGACY_RECORD_READS").unwrap() == "true",
        );
        match step.as_str() {
            "write-normalized" | "write-historical" => {
                let rows = write_rows(&keys, step == "write-normalized").await;
                std::fs::write(file, bson::to_vec(&rows).unwrap()).unwrap();
            }
            "read" => {
                read_rows(
                    &keys,
                    &bson::from_slice(&std::fs::read(file).unwrap()).unwrap(),
                )
                .await
            }
            _ => panic!("unexpected catalog fixture step"),
        }
    } else {
        for contextual in [false, true] {
            keys.set_contextual_record_writes(contextual);
            keys.set_allow_legacy_record_reads(true);
            let rows = write_rows(&keys, contextual).await;
            read_rows(&keys, &rows).await;
            keys.set_contextual_record_writes(true);
            keys.set_allow_legacy_record_reads(false);
            read_rows(&keys, &rows).await;
        }
    }
}

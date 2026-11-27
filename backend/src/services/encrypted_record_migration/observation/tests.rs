use super::*;
use crate::crypto::{
    record_field::{
        self,
        authenticated::{Format, LogicalKey, Slot},
    },
    token::hash_token,
};
use crate::test_utils::{test_app_config, test_encryption_keys};
use bson::{Binary, DateTime, doc, spec::BinarySubtype};
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

fn family(collection: &str, first: &str) -> Family {
    families()
        .find(|f| f.collection() == collection && f.entry.fields[0] == first)
        .unwrap()
}
fn bin(bytes: Vec<u8>) -> Bson {
    Bson::Binary(Binary {
        subtype: BinarySubtype::Generic,
        bytes,
    })
}

async fn seal(
    keys: &EncryptionKeys,
    coll: &str,
    id: &str,
    owner: &str,
    path: &[&str],
    plain: &[u8],
) -> Vec<u8> {
    record_field::encrypt_context(
        keys,
        &RecordContext::new(coll, id, owner, path).unwrap(),
        plain,
    )
    .await
    .unwrap()
}

fn materials(obs: &Observation) -> impl Iterator<Item = (&'static str, Marker, &Evidence)> {
    obs.instances
        .iter()
        .filter_map(|(label, storage)| match storage {
            Storage::Encrypted {
                marker, evidence, ..
            } => Some((*label, *marker, evidence)),
            _ => None,
        })
}

async fn inspect(f: &Family, k: &EncryptionKeys, row: &Document) -> Observation {
    let before = bson::to_vec(row).unwrap();
    let counters = k.decrypt_stats();
    let result = f.observe(k, row, &Default::default()).await;
    assert_eq!(before, bson::to_vec(row).unwrap());
    assert_eq!(counters, k.decrypt_stats());
    result
}

// Construct the documented wire contracts independently of the observation branch.
async fn fixture(f: &Family, keys: &EncryptionKeys, current: bool) -> Document {
    use crate::services::{
        oauth_broker_record_service as broker, oauth_state_record_service as oauth,
    };
    let coll = f.collection();
    let field = f.entry.fields[0];
    let id = hash_token("observation-fixture");
    let mut row = doc! {"_id":&id,"user_id":"owner","created_by":"owner","provider_config_id":"provider","client_id":"client","service_id":"service","expires_at":DateTime::from_millis(0),"created_at":DateTime::from_millis(0)};
    let mut versions = Document::new();
    match f.entry.adapter {
        Adapter::Header => {
            let value = if current {
                let bytes = seal(
                    keys,
                    coll,
                    &id,
                    "owner",
                    &[field, "x-example", "value", "visibility", "public"],
                    b"value",
                )
                .await;
                doc! {"name":"X-Example","value_encrypted":bin(bytes),"value_context_version":4}
            } else {
                doc! {"name":"X-Example","value":"legacy"}
            };
            row.insert(field, vec![value]);
        }
        Adapter::PlatformCredential => {
            row.extend(
                doc! {"provider":"example","updated_by":"owner","updated_at":DateTime::now()},
            );
            row.insert(
                field,
                doc! {"alpha":bin(seal(keys,coll,&id,"platform",&[field,"alpha"],b"value").await)},
            );
            if current {
                versions.insert("alpha", 3);
            }
        }
        Adapter::CatalogCa => {
            let ca =
                ssh_key::PrivateKey::random(&mut rand::rngs::OsRng, ssh_key::Algorithm::Ed25519)
                    .unwrap();
            let public = ca.public_key().to_openssh().unwrap();
            let bytes = crate::services::ssh_ca_record_service::encrypt(
                keys,
                &id,
                &public,
                ca.to_openssh(ssh_key::LineEnding::LF).unwrap().as_bytes(),
            )
            .await
            .unwrap();
            let mut config = doc! {"host":"host","port":22,"ca_public_key":public,"ca_private_key_encrypted":bin(bytes)};
            if current {
                config.insert("ca_context_version", 3);
            }
            row.insert("ssh_config", config);
        }
        Adapter::OAuthState => {
            let state = bson::from_document(row.clone()).unwrap();
            for &name in f.entry.fields {
                row.insert(
                    name,
                    oauth::encrypt(keys, &state, name, b"fixture")
                        .await
                        .unwrap(),
                );
            }
            // Expired continuations remain dependencies until MongoDB removes the row.
            let continuation = uuid::Uuid::new_v4().to_string();
            let callback_keys = test_encryption_keys();
            let bytes = seal(
                &callback_keys,
                coll,
                &id,
                "owner",
                &["pending_callback", "provider", &continuation],
                b"code",
            )
            .await;
            row.insert("pending_callback",doc!{"id":continuation,"encrypted_code":hex::encode(bytes),"expires_at":DateTime::from_millis(0)});
        }
        Adapter::Broker => {
            row.insert(
                field,
                bin(broker::encrypt(keys, &id, "owner", "client", b"token")
                    .await
                    .unwrap()),
            );
            if current {
                versions.insert(field, 3);
            }
        }
        _ => {
            for &name in f.entry.fields {
                let owner = match f.entry.adapter {
                    Adapter::OAuthWebhook => "creator:owner",
                    _ if coll == "provider_configs" => "platform",
                    _ => "owner",
                };
                let clear = match f.entry.adapter {
                    Adapter::Mfa => {
                        row.insert("factor_type", "totp");
                        "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP".into()
                    }
                    Adapter::Node => {
                        let s = "2a".repeat(32);
                        row.insert("signing_secret_hash", hash_token(&s));
                        s
                    }
                    Adapter::SavedLogin if name == "totp_secret_encrypted" => {
                        "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP".into()
                    }
                    _ => "value".to_owned(),
                };
                let mut path = vec![name];
                if matches!(f.entry.adapter, Adapter::Connection) {
                    path.extend(["service_id", "service"]);
                }
                let bytes = seal(keys, coll, &id, owner, &path, clear.as_bytes()).await;
                // Exercise the pre-Binary integer-array representation.
                row.insert(
                    name,
                    Bson::Array(
                        bytes
                            .into_iter()
                            .map(|b| Bson::Int64(i64::from(b)))
                            .collect(),
                    ),
                );
                if current {
                    match f.entry.adapter {
                        Adapter::Mfa | Adapter::SavedLogin => {
                            row.insert("secret_context_version", 3);
                        }
                        Adapter::Node => {
                            row.insert("signing_secret_context_version", 3);
                        }
                        _ => {
                            versions.insert(name, 3);
                        }
                    }
                }
            }
        }
    }
    if !versions.is_empty() {
        row.insert(credentials::VERSIONS, versions);
    }
    row
}

#[tokio::test]
async fn registry_matrix_preserves_bson_and_traffic_on_current_historical_absent_and_invalid_sources()
 {
    let mut reader = test_encryption_keys();
    reader.set_allow_legacy_record_reads(false);
    let mut seen = BTreeSet::new();
    for f in families().filter(|f| {
        super::channels::source_companions(f.entry).is_none()
            && super::retained::source_companions(f.entry).is_none()
            && super::deliveries::source_companions(f.entry).is_none()
            && super::nested::source_companions(f.entry).is_none()
    }) {
        assert!(seen.insert((f.collection(), f.entry.fields[0])));
        assert!(!f.source_companions.is_empty());
        for current in [true, false] {
            let mut keys = test_encryption_keys();
            keys.set_contextual_record_writes(current);
            let row = fixture(&f, &keys, current).await;
            let observed = inspect(&f, &reader, &row).await;
            assert!(
                observed
                    .findings
                    .iter()
                    .all(|(_, r)| *r == Finding::AmbiguousBinding),
                "{:?} {:?}",
                f.entry.adapter,
                observed.findings
            );
            assert_eq!(
                f.fields().collect::<BTreeSet<_>>(),
                observed.instances.iter().map(|p| p.0).collect()
            );
            for (label, _, ev) in materials(&observed) {
                assert_eq!(ev.route, Route::Primary(Slot::Current, LogicalKey::Local));
                assert_eq!(
                    ev.format,
                    if current || label == CALLBACK {
                        Format::V3
                    } else {
                        Format::V2
                    }
                );
            }
            if current {
                assert_eq!(materials(&observed).count(), f.fields().count());
            }
            if current {
                let mut damaged = row.clone();
                let field = f.entry.fields[0];
                let invalid_bytes = bin(vec![0]);
                match f.entry.adapter {
                    Adapter::Header => {
                        let Some(Bson::Document(first)) =
                            damaged.get_array_mut(field).unwrap().first_mut()
                        else {
                            unreachable!()
                        };
                        first.insert("value_encrypted", invalid_bytes);
                    }
                    Adapter::PlatformCredential => {
                        damaged
                            .get_document_mut(field)
                            .unwrap()
                            .insert("alpha", invalid_bytes);
                    }
                    Adapter::CatalogCa => {
                        damaged
                            .get_document_mut("ssh_config")
                            .unwrap()
                            .insert("ca_private_key_encrypted", invalid_bytes);
                    }
                    Adapter::OAuthState => {
                        damaged.insert(field, "v3:00");
                    }
                    _ => {
                        damaged.insert(field, invalid_bytes);
                    }
                }
                let partial = inspect(&f, &reader, &damaged).await;
                assert_eq!(partial.findings, [(field, Finding::Authentication)]);
                assert_eq!(materials(&partial).count() + 1, f.fields().count());
            }
            let mut corrupt = row.clone();
            corrupt.insert("_id", 42);
            assert!(!inspect(&f, &reader, &corrupt).await.findings.is_empty());
            let mut absent = row;
            for &field in f.entry.fields {
                absent.remove(field.split('.').next().unwrap());
            }
            for field in [
                "secret_context_versions",
                "secret_context_version",
                "signing_secret_context_version",
                "pending_callback",
            ] {
                absent.remove(field);
            }
            let missing = inspect(&f, &reader, &absent).await;
            assert_eq!(materials(&missing).count(), 0);
            assert_eq!(
                !missing.findings.is_empty(),
                matches!(
                    f.entry.adapter,
                    Adapter::CatalogCredential
                        | Adapter::Mfa
                        | Adapter::SavedLogin
                        | Adapter::Broker
                ),
                "{:?}",
                f.entry.adapter
            );
        }
    }
    assert_eq!(seen.len(), 16);
}

#[tokio::test]
async fn header_versions_owner_transfer_and_plaintext_share_a_snapshot_without_service_effects() {
    use crate::services::default_header_service;
    use mongodb::event::{EventHandler, command::CommandEvent};
    let commands = Arc::new(AtomicUsize::new(0));
    let seen = commands.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "header_census",
        EventHandler::callback(move |e| {
            if matches!(e, CommandEvent::Started(_)) {
                seen.fetch_add(1, Ordering::SeqCst);
            }
        }),
    )
    .await;
    let keys = test_encryption_keys();
    let field = "default_request_headers";
    let f = family("downstream_services", field);
    let mut row = fixture(&f, &keys, true).await;
    let id = row.get_str("_id").unwrap().to_owned();
    let old = seal(
        &keys,
        f.collection(),
        &id,
        "owner",
        &[field, "x-old", "value"],
        b"old",
    )
    .await;
    row.get_array_mut(field).unwrap().extend([
        Bson::Document(doc!{"name":"X-Old","sensitive":true,"value_context_version":3,"value_encrypted":bin(old)}),
        Bson::Document(doc!{"name":"X-Plain","value":"visible"})]);
    row.insert("owner_user_id", Bson::Null);
    let rows = db.collection::<Document>(f.collection());
    rows.insert_one(&row).await.unwrap();
    let stored = rows.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
    let before = commands.load(Ordering::SeqCst);
    let observed = inspect(&f, &keys, &stored).await;
    assert_eq!(before, commands.load(Ordering::SeqCst));
    assert!(observed.findings.is_empty());
    assert_eq!(
        materials(&observed)
            .map(|(_, m, e)| (m, e.format))
            .collect::<Vec<_>>(),
        [
            (Marker::Version(4), Format::V3),
            (Marker::Version(3), Format::V3)
        ]
    );
    assert!(matches!(observed.instances[2].1, Storage::Plaintext));
    let mut transferred = stored.clone();
    transferred.insert("owner_user_id", "new-owner");
    assert_eq!(
        materials(&inspect(&f, &keys, &transferred).await).count(),
        0
    );
    let headers = bson::from_bson::<Vec<_>>(stored.get(field).unwrap().clone()).unwrap();
    let next = default_header_service::rebind(
        Some(&keys),
        f.collection(),
        &id,
        "owner",
        "new-owner",
        Some(&headers),
    )
    .await
    .unwrap();
    transferred.insert(field, bson::to_bson(&next).unwrap());
    let result = inspect(&f, &keys, &transferred).await;
    assert!(result.findings.is_empty());
    assert_eq!(materials(&result).count(), 3);
    assert_eq!(
        bson::to_vec(&stored).unwrap(),
        bson::to_vec(&rows.find_one(doc! {"_id":&id}).await.unwrap().unwrap()).unwrap()
    );
    assert!(
        default_header_service::migrate_snapshot(&db, &keys, f.collection(), &stored, "owner")
            .await
            .unwrap()
    );
    let migrated = rows.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
    default_header_service::verify_current_snapshot(&keys, f.collection(), &migrated, "owner")
        .await
        .unwrap();
    db.drop().await.unwrap();
}

#[tokio::test]
async fn map_counts_duplicate_material_mixed_routes_and_damaged_siblings_independently() {
    let mut config = test_app_config();
    let old_key = "22".repeat(32);
    config.encryption_key_previous = Some(old_key.clone());
    let reader = EncryptionKeys::from_config(&config);
    config.encryption_key = Some(old_key);
    config.encryption_key_previous = None;
    let old = EncryptionKeys::from_config(&config);
    let f = family("platform_credentials", "secrets");
    let mut row = fixture(&f, &reader, true).await;
    let id = row.get_str("_id").unwrap();
    let bytes = seal(
        &old,
        f.collection(),
        id,
        "platform",
        &["secrets", "beta"],
        b"value",
    )
    .await;
    let secrets = row.get_document_mut("secrets").unwrap();
    secrets.insert("beta", bin(bytes));
    secrets.insert("broken", true);
    row.get_document_mut(credentials::VERSIONS)
        .unwrap()
        .insert("beta", 3);
    let o = inspect(&f, &reader, &row).await;
    assert_eq!(o.findings, [("secrets", Finding::Malformed)]);
    let routes: Vec<_> = materials(&o).map(|(_, _, e)| &e.route).collect();
    assert_eq!(
        routes,
        [
            &Route::Primary(Slot::Current, LogicalKey::Local),
            &Route::Primary(Slot::Previous, LogicalKey::Local)
        ]
    );
}

#[tokio::test]
async fn content_checks_and_missing_material_keep_distinct_evidence_after_authentication() {
    let keys = test_encryption_keys();
    let node = family("nodes", "signing_secret_encrypted");
    let mut row = fixture(&node, &keys, true).await;
    row.insert("signing_secret_hash", hash_token("other"));
    let result = inspect(&node, &keys, &row).await;
    assert_eq!(
        result.findings,
        [("signing_secret_encrypted", Finding::InvalidValue)]
    );
    assert_eq!(materials(&result).count(), 1);

    let mfa = family("mfa_factors", "secret_encrypted");
    let mut row = fixture(&mfa, &keys, true).await;
    row.insert("factor_type", "recovery_codes");
    let result = inspect(&mfa, &keys, &row).await;
    assert_eq!(materials(&result).count(), 1);
    assert_eq!(
        result.findings,
        [("secret_encrypted", Finding::InvalidValue)]
    );
    row.remove("secret_encrypted");
    row.remove("secret_context_version");
    assert!(inspect(&mfa, &keys, &row).await.findings.is_empty());

    let broker = family("oauth_broker_bindings", "refresh_token_encrypted");
    let mut old = test_encryption_keys();
    old.set_contextual_record_writes(false);
    let mut row = fixture(&broker, &old, false).await;
    assert!(inspect(&broker, &keys, &row).await.findings.is_empty());
    row.insert("_id", hash_token("other-binding"));
    assert_eq!(
        inspect(&broker, &keys, &row).await.findings,
        [("refresh_token_encrypted", Finding::Authentication)]
    );
    row.remove("refresh_token_encrypted");
    row.insert("revoked", true);
    assert!(inspect(&broker, &keys, &row).await.findings.is_empty());

    let catalog = family("downstream_services", "credential_encrypted");
    let mut row = fixture(&catalog, &keys, true).await;
    row.insert("owner_user_id", "transferred");
    assert_eq!(materials(&inspect(&catalog, &keys, &row).await).count(), 0);
    row.insert("owner_user_id", bson::Bson::Null);
    assert_eq!(materials(&inspect(&catalog, &keys, &row).await).count(), 1);
    row.insert("credential_encrypted", Bson::Array(vec![]));
    row.remove(credentials::VERSIONS);
    assert!(matches!(
        inspect(&catalog, &keys, &row).await.instances[0].1,
        Storage::Sentinel
    ));
    let bytes = crate::services::catalog_credential_record_service::empty_credential(
        &old,
        row.get_str("_id").unwrap(),
        "owner",
    )
    .await
    .unwrap();
    row.insert("credential_encrypted", bin(bytes));
    assert!(matches!(
        inspect(&catalog, &keys, &row).await.instances[0].1,
        Storage::Encrypted { empty: true, .. }
    ));
}
mod channels;
mod deliveries;
mod nested;
mod retained;

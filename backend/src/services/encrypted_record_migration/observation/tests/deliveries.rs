use super::super::companions::Parents;
use super::{retained::check, *};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

fn jwt(kind: &str) -> String {
    let body = serde_json::json!({"sub":"owner","sid":"session","token_type":kind,"jti":"issued"});
    format!(
        "fixture.{}.fixture",
        URL_SAFE_NO_PAD.encode(body.to_string())
    )
}

#[tokio::test]
async fn delivery_registry_includes_expired_unpurged_pairs_and_conflicting_grants() {
    let mut reader = test_encryption_keys();
    reader.set_allow_legacy_record_reads(false);
    let expired = DateTime::from_millis(0);
    let parents:Parents=[
        ("api_keys",doc!{"user_id":"owner","key_hash":hash_token("api-key"),"platform":"device-code","allowed_node_ids":["node"]}),
        ("nodes",doc!{"user_id":"owner","metadata":{"provisioning_source":"device-code"}}),
        ("api_key_credentials",doc!{"api_key_id":"key","login_request_id":"row","secret_hash":hash_token("grant")})
    ].into();
    let mut seen = BTreeSet::new();
    for f in families().filter(|f| super::super::deliveries::source_companions(f.entry).is_some()) {
        for current in [true, false] {
            let mut k = test_encryption_keys();
            k.set_contextual_record_writes(current);
            let mut row = doc! {"_id":"row","status":"approved","created_at":expired,"expires_at":expired,
            "poll_interval_secs":5,"slow_down_increments":0,"device_code_hmac":"d","user_code_hmac":"u","approved_user_id":"owner"};
            if current {
                row.insert("delivery_context_version", 3);
            }
            let mut child = None;
            for &field in f.entry.fields {
                seen.insert((f.collection(), field));
                let bytes = match f.entry.adapter {
                    Adapter::NodeDevice => {
                        let signer = vec![9_i32; 32];
                        let encoded = "09".repeat(32);
                        let hash = hash_token("refresh");
                        row.extend(doc!{"status":"expired","device_pubkey":signer,"device_code_hash":"code","hw_id":"hw","issued_api_key_id":"key","issued_node_id":"node","refresh_token_hash":&hash,"approved_by_user_id":"owner","last_rotated_at":expired,"user_code_history":[],"failed_poll_count":0});
                        let plain = if field == "delivery_api_key_encrypted" {
                            b"api-key".as_slice()
                        } else {
                            b"refresh".as_slice()
                        };
                        seal(
                            &k,
                            f.collection(),
                            "row",
                            "owner",
                            &[
                                field, "personal", "owner", "key", "node", "code", &encoded, &hash,
                            ],
                            plain,
                        )
                        .await
                    }
                    Adapter::AgentLogin => {
                        row.extend(doc! {"api_key_id":"key","credential_id":"child"});
                        seal(
                            &k,
                            f.collection(),
                            "row",
                            "owner",
                            &[field, "key", "child"],
                            b"grant",
                        )
                        .await
                    }
                    Adapter::AuthDevice if field.contains('.') => {
                        child = Some(
                            doc! {"key_was_created":false,"api_key_id":"key","credential_id":"child","delivery_credential_encrypted":bin(seal(&k,f.collection(),"row","owner",&[field,"key","child"],b"grant").await)},
                        );
                        continue;
                    }
                    Adapter::AuthDevice => {
                        row.insert("approved_session_id", "session");
                        let kind = if field.contains("access") {
                            "access"
                        } else {
                            "refresh"
                        };
                        seal(
                            &k,
                            f.collection(),
                            "row",
                            "owner",
                            &[field, "session"],
                            jwt(kind).as_bytes(),
                        )
                        .await
                    }
                    _ => unreachable!(),
                };
                row.insert(field, bin(bytes));
            }
            let out = check(&reader, &f, &row, &parents).await;
            let n = if matches!(f.entry.adapter, Adapter::AgentLogin) {
                1
            } else {
                2
            };
            assert_eq!(materials(&out).count(), n, "{}", f.collection());
            assert!(
                out.findings
                    .iter()
                    .all(|(_, v)| *v == Finding::AmbiguousBinding && !current),
                "{} {:?}",
                f.collection(),
                out.findings
            );
            if matches!(f.entry.adapter, Adapter::AuthDevice) && !current {
                assert_eq!(
                    out.refresh_jti.as_deref().map(String::as_str),
                    Some("issued")
                );
            }
            if let Some(grant) = child {
                let mut mixed = row.clone();
                mixed.insert("agent_key_grant", &grant);
                let out = check(&reader, &f, &mixed, &parents).await;
                assert_eq!(materials(&out).count(), 3);
                assert!(out.findings.iter().any(|(_, v)| *v == Finding::Malformed));
                for k in [
                    "approved_session_id",
                    "delivery_access_token_encrypted",
                    "delivery_refresh_token_encrypted",
                ] {
                    mixed.remove(k);
                }
                let grant_only = check(&reader, &f, &mixed, &parents).await;
                assert_eq!(materials(&grant_only).count(), 1);
                assert!(grant_only.findings.is_empty());
            }
            let label = f
                .entry
                .fields
                .iter()
                .copied()
                .find(|k| !k.contains('.'))
                .unwrap();
            let mut bad = row.clone();
            bad.insert(label, "invalid BSON");
            let partial = check(&reader, &f, &bad, &parents).await;
            assert!(!partial.findings.is_empty());
            assert_eq!(materials(&partial).count(), n - 1);
            let mut absent = row.clone();
            for &key in f.entry.fields {
                absent.remove(key);
            }
            absent.remove("delivery_context_version");
            absent.insert("status", "pending");
            absent.remove("approved_session_id");
            let empty = check(&reader, &f, &absent, &parents).await;
            assert!(empty.findings.is_empty());
            assert_eq!(materials(&empty).count(), 0);
        }
    }
    assert_eq!(seen.len(), 8);
}

#[tokio::test]
async fn historical_refresh_issuance_is_checked_at_the_observed_cut() {
    use crate::services::encrypted_record_census::{Census, Scope};
    let db = crate::test_utils::connect_transaction_test_database("census_refresh_jti").await;
    crate::services::encrypted_record_census::tests::with_history(&db, 120, async {

    let f = family("auth_device_codes", "delivery_access_token_encrypted");
    let k = test_encryption_keys();
    let mut old = test_encryption_keys();
    old.set_contextual_record_writes(false);
    let now = DateTime::from_millis(0);
    let mut row = doc! {"_id":"child","status":"approved","approved_user_id":"owner","approved_session_id":"session","created_at":now,"expires_at":now,"device_code_hmac":"d","user_code_hmac":"u","slow_down_increments":0,"poll_interval_secs":5};
    for (&field, kind) in f.entry.fields.iter().zip(["access", "refresh"]) {
        row.insert(
            field,
            bin(seal(
                &old,
                f.collection(),
                "child",
                "owner",
                &[field, "session"],
                jwt(kind).as_bytes(),
            )
            .await),
        );
    }
    db.collection::<Document>(f.collection())
        .insert_one(&row)
        .await
        .unwrap();
    db.create_collection("refresh_tokens").await.unwrap();
    db.create_collection("api_key_credentials").await.unwrap();
    let scope = Scope {
        registry_digest: "fixture".into(),
        restore_epoch: "original".into(),
        deployment_revision: "one".into(),
        namespaces: [f.collection(), "refresh_tokens", "api_key_credentials"]
            .map(str::to_owned)
            .into(),
    };
    let (census, start) = Census::start(&db, &k, scope).await.unwrap();
    let out = f
        .observe_at(&k, &row, &census.snapshot(&start).await.unwrap())
        .await
        .unwrap();
    assert_eq!(
        out.findings,
        vec![("delivery_refresh_token_encrypted", Finding::MissingParent)]
    );
    assert_eq!(f.rescan_parents(), ["refresh_tokens"]);
    db.collection::<Document>("refresh_tokens")
        .insert_one(
            doc! {"_id":"independent","jti":"issued","user_id":"owner","session_id":"session"},
        )
        .await
        .unwrap();
    census.request_cut().await.unwrap();
    let cut = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if let Some(c) = census.capture().await.unwrap() {
                break c;
            }
        }
    })
    .await
    .unwrap();
    let out = f
        .observe_at(&k, &row, &census.snapshot(&cut).await.unwrap())
        .await
        .unwrap();
    assert!(out.findings.is_empty());
    assert!(out.refresh_jti.is_none());
    assert_eq!(materials(&out).count(), 2);
    }).await;
    db.drop().await.unwrap();
}

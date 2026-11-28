use super::*;
use crate::services::{
    nyxbot_response_recovery as replay, platform_entry_gateway_service as gateway,
};

// Each fixture selects the persisted field and binding independently of the observer.
async fn fixture(f: &Family, k: &EncryptionKeys, current: bool) -> Document {
    let id = uuid::Uuid::new_v4().to_string();
    let now = DateTime::from_millis(0);
    let field = f.entry.fields[0];
    let mut row = doc! {"_id":&id,"user_id":"owner","channel_id":"chan","created_at":now,"updated_at":now,"expires_at":now};
    let (path, plain, marker): (Vec<&str>, Vec<u8>, &str) = match f.entry.adapter {
        Adapter::ThreadReference => {
            row.insert("partition", "p");
            (
                vec![field, "chan"],
                b"reference".to_vec(),
                "event_ref_context_version",
            )
        }
        Adapter::NyxbotEvent => {
            row.insert("event_id", "event");
            row.insert(
                "event_context_ciphertext",
                bin(seal(
                    k,
                    f.collection(),
                    &id,
                    "owner",
                    &["event_context_ciphertext", "chan"],
                    br#"{"activity":{"event_id":"event"}}"#,
                )
                .await),
            );
            if current {
                row.insert("event_context_version", 3);
            }
            let target = seal(
                k,
                f.collection(),
                &id,
                "owner",
                &[field, "chan"],
                b"address",
            )
            .await;
            let mut delivery = doc! {"target_ciphertext":bin(target),"target_expires_at":now};
            if current {
                delivery.insert("target_context_version", 3);
            }
            row.insert("delivery", delivery);
            return row;
        }
        Adapter::GatewayResponse => {
            row.extend(doc!{"response_id":"response","binding_id":"binding","partition":"p","event_key":"event","request_hash":"hash","terminal":false,"revision":0});
            let payload = replay::Payload::new(
                "response",
                &replay::conversation_identity("chan", "binding", "p"),
                Default::default(),
            );
            (
                vec![field, "chan", "binding", "response"],
                serde_json::to_vec(&payload).unwrap(),
                "payload_context_version",
            )
        }
        Adapter::GatewayTransport => {
            let p = b" {\"request\": true } ";
            row.extend(doc!{"profile_id":"profile","configuration":{"identity":{"version":2,"platform":"discord","channel_bot_id":"bot","platform_bot_id":"external"},"transport":{"kind":"direct"},"transport_generation":1,"credential_fingerprint":"digest","control_surface":{"kind":"private_message"}},"configured_by":"actor","credential_owner_id":"owner","route_api_key_id":"route","route_id":"route","provider_api_key_id":"key","callback_url":"uri","gateway_base":"uri","setup_state":"done","setup_attempt_id":"attempt","setup_dispatch_deadline":Bson::Null,"request_policy":{},"request_hash":gateway::sha256(p),"creation_fingerprint":"fingerprint","state":"active","revision":1});
            (vec![field], p.to_vec(), "")
        }
        Adapter::Setup => {
            row.extend(doc!{"owner_user_id":"owner","source":"owner","idempotency_key":"key","idempotency_scope":"scope","template":"restaurant-starter","template_version":1,"revision":1,"authority_generation":1,"state":"draft","limits":{"daily_credits":1,"turns_per_hour":1,"concurrent_turns":1,"max_input_chars":32,"max_output_tokens":32},"resources":{},"delivery_tested":false});
            (
                vec![field, "scope"],
                serde_json::to_vec(&crate::models::platform_setup::SetupFacts {
                    name: "Cafe".into(),
                    timezone: "UTC".into(),
                    ..Default::default()
                })
                .unwrap(),
                "facts_context_version",
            )
        }
        Adapter::Trigger if f.collection() == "triggers" => {
            row.extend(doc!{"label":"trigger","status":"active","secret_hash":hash_token("verify"),"verification":{"mode":"hmac_sha256","header_name":"X-Test"},"delivery":{"type":"notification"}});
            (vec![field], b"verify".to_vec(), "")
        }
        Adapter::Trigger => {
            row.extend(
                doc! {"trigger_id":"trigger","event_id":"event","attempts":0,"status":"pending"},
            );
            (
                vec![field, "trigger_id", "trigger", "event_id", "event"],
                br#"{"trigger_id":"trigger","event_id":"event","payload":{}}"#.to_vec(),
                "",
            )
        }
        _ => unreachable!(),
    };
    row.insert(
        field,
        bin(seal(k, f.collection(), &id, "owner", &path, &plain).await),
    );
    if current {
        if marker.is_empty() {
            row.insert("secret_context_versions", doc! {field:3});
        } else {
            row.insert(marker, 3);
        }
    }
    row
}

#[tokio::test]
async fn replay_setup_trigger_and_transport_registry_matrix() {
    let mut reader = test_encryption_keys();
    reader.set_allow_legacy_record_reads(false);
    let mut seen = 0;
    for f in families().filter(|f| {
        super::super::protocols::source_companions(f.entry).is_some()
            && !matches!(
                f.entry.adapter,
                Adapter::OracleLoginOuter
                    | Adapter::NativeReplyHandle
                    | Adapter::MobileSocialDelivery
            )
    }) {
        seen += 1;
        for current in [false, true] {
            let mut k = test_encryption_keys();
            k.set_contextual_record_writes(current);
            let mut row = fixture(&f, &k, current).await;
            let out = inspect(&f, &reader, &row).await;
            assert!(
                out.findings
                    .iter()
                    .all(|(_, r)| !current && *r == Finding::AmbiguousBinding),
                "{} {:?}",
                f.collection(),
                out.findings
            );
            let n = if matches!(f.entry.adapter, Adapter::NyxbotEvent) {
                2
            } else {
                1
            };
            assert_eq!(materials(&out).count(), n, "{}", f.collection());
            if matches!(f.entry.adapter, Adapter::NyxbotEvent) {
                row.get_document_mut("delivery")
                    .unwrap()
                    .insert("target_ciphertext", bin(vec![0]));
            } else {
                row.insert(f.entry.fields[0], bin(vec![0]));
            }
            let bad = inspect(&f, &reader, &row).await;
            assert_eq!(materials(&bad).count(), n - 1);
            assert!(!bad.findings.is_empty());
            let empty = inspect(&f, &reader, &doc! {"_id":"missing"}).await;
            assert_eq!(materials(&empty).count(), 0);
            assert!(
                !empty.findings.is_empty()
                    || matches!(
                        f.entry.adapter,
                        Adapter::ThreadReference | Adapter::NyxbotEvent
                    )
            );
        }
    }
    assert_eq!(seen, 7);
}

#[tokio::test]
async fn oracle_counts_only_the_authenticated_outer_and_dates_the_cleared_profile_at_the_cut() {
    use crate::crypto::record_field::oracle::OracleOuter;
    let k = test_encryption_keys();
    let now = DateTime::from_millis(5000);
    for f in families().filter(|f| matches!(f.entry.adapter, Adapter::OracleLoginOuter)) {
        let id = uuid::Uuid::new_v4().to_string();
        let pool = uuid::Uuid::new_v4().to_string();
        let sealed_inner = b"opaque recipient bytes\0\xff";
        let field = f.entry.fields[0];
        let mut row = doc! {"_id":&id,"pool_id":&pool,"format_version":1,"created_at":now,"updated_at":now,"expires_at":now,"created_by_user_id":"operator","name":"profile","generation":uuid::Uuid::new_v4().to_string(),"revision":uuid::Uuid::new_v4().to_string(),"worker_token_hash":"hash","bindings":[],"envelope_size":sealed_inner.len() as i64,field:bin(vec![])};
        let profile = f.collection() == "oracle_login_profiles";
        let outer = if profile {
            OracleOuter::profile(&bson::from_document(row.clone()).unwrap())
        } else {
            OracleOuter::snapshot(&pool, &id, 1)
        };
        row.insert(field, bin(outer.seal(&k, sealed_inner).await.unwrap()));
        let out = inspect(&f, &k, &row).await;
        assert!(out.findings.is_empty());
        assert!(matches!(
            out.instances.as_slice(),
            [(
                _,
                Storage::Encrypted {
                    contract: Contract::OracleOuter,
                    evidence: Evidence {
                        format: Format::V2,
                        ..
                    },
                    ..
                }
            )]
        ));
        row.insert("envelope_size", 1);
        let bad = inspect(&f, &k, &row).await;
        assert!(bad.findings.contains(&(field, Finding::InvalidValue)));
        assert_eq!(materials(&bad).count(), 1);
        row.insert(field, bin(vec![]));
        row.insert("envelope_size", 0);
        for time in [4, 6] {
            let out = f
                .observe_at_time(
                    &k,
                    &row,
                    &Default::default(),
                    chrono::DateTime::from_timestamp(time, 0).unwrap(),
                )
                .await;
            assert_eq!(out.findings.is_empty(), profile && time == 6);
        }
    }
}

#[test]
fn native_receipt_and_mobile_delivery_use_the_retained_contract_without_redemption() {
    use crate::models::platform_entry_identity::NATIVE_RECEIPTS;
    use crate::services::{
        mobile_social_service, platform_native_reply_service::tests::discord_fixture_with_keys,
        token_service,
    };
    crate::test_utils::run_native_journey_test(
        "observe_receipts_and_mobile_deliveries",
        || async {
            let server = wiremock::MockServer::start().await;
            let mut reader = test_encryption_keys();
            reader.set_allow_legacy_record_reads(false);
            for current in [false, true] {
                let mut k = test_encryption_keys();
                k.set_contextual_record_writes(current);
                let (state, _, receipt) = discord_fixture_with_keys(&server.uri(), k).await;
                let f = family("platform_native_reply_handles", "ciphertext");
                let row = state
                    .db
                    .collection::<Document>(f.collection())
                    .find_one(doc! {"receipt_id":&receipt.id})
                    .await
                    .unwrap()
                    .unwrap();
                let parent = bson::to_document(&receipt).unwrap();
                let observed = retained::check(
                    &reader,
                    &f,
                    &row,
                    &[(NATIVE_RECEIPTS, parent.clone())].into(),
                )
                .await;
                assert!(observed.findings.is_empty());
                assert_eq!(materials(&observed).count(), 1);
                assert!(
                    inspect(&f, &reader, &row)
                        .await
                        .findings
                        .contains(&("ciphertext", Finding::MissingParent))
                );
                let mut changed = parent;
                changed.insert("content_commitment", "mismatch");
                assert!(
                    !retained::check(&reader, &f, &row, &[(NATIVE_RECEIPTS, changed)].into())
                        .await
                        .findings
                        .is_empty()
                );

                let user = uuid::Uuid::new_v4().to_string();
                let prepared = token_service::prepare_session_tokens(
                    &state.db,
                    &state.config,
                    &state.jwt_keys,
                    &user,
                    None,
                    None,
                )
                .await
                .unwrap();
                let f = family("mobile_social_deliveries", "ciphertext");
                mobile_social_service::issue_code(
                    &state.db,
                    &state.encryption_keys,
                    prepared,
                    &user,
                    &"A".repeat(43),
                )
                .await
                .unwrap();
                let rows = state.db.collection::<Document>(f.collection());
                let row = rows
                    .find_one(doc! {"user_id":&user})
                    .await
                    .unwrap()
                    .unwrap();
                let out = inspect(&f, &reader, &row).await;
                assert!(out.findings.is_empty());
                assert_eq!(materials(&out).count(), 1);
                let mut swapped = row.clone();
                swapped.insert("code_hash", hash_token("other"));
                assert_eq!(materials(&inspect(&f, &reader, &swapped).await).count(), 0);
                assert_eq!(
                    rows.find_one(doc! {"user_id":&user})
                        .await
                        .unwrap()
                        .unwrap(),
                    row
                );
                state.db.drop().await.unwrap();
            }
        },
    );
}

use super::super::companions::Parents;
use super::*;
use crate::crypto::record_field::historical::HistoricalAad;
use sha2::{Digest, Sha256};

fn parent_rows() -> Parents {
    [
        (
            "api_keys",
            doc! {"_id":"key","user_id":"owner","key_hash":hash_token("value")},
        ),
        (
            "platform_public_sessions",
            doc! {"_id":"session","sponsor_owner_user_id":"owner"},
        ),
    ]
    .into()
}

async fn retained_fixture(f: &Family, k: &EncryptionKeys, current: bool) -> (Document, Document) {
    let now = DateTime::now();
    let mut row = doc! {"_id":"row","user_id":"owner","api_key_id":"key","agent_id":"agent","conversation_id":"chat","turn_id":"turn","created_at":now,"expires_at":now,"state":"pending"};
    let mut absent = Document::new();
    let mut versions = Document::new();
    for &label in f.entry.fields {
        let mut plain = b"value".to_vec();
        let mut path = vec![label];
        let mut marker = "secret_context_versions";
        match f.entry.adapter {
            Adapter::AssistantCredential => {
                row.insert("last_used_at", now);
                path.extend(["conversation", "bound", "chat", "api_key", "key"]);
            }
            Adapter::Attachment => {
                row.extend(doc!{"origin":"user_upload","content_type":"text/plain","label":"hello","pages":1,"chunks":1,"size":5_i64});
                if label == "data_encrypted" {
                    continue;
                }
                plain = br#"{"content_type":"text/plain","text":"hello","sections":[]}"#.to_vec();
            }
            Adapter::ToolResult => {
                row.extend(doc!{"version":1,"actor_id":"actor","guest":false,"key_authority_fence":0_i64,"size":5_i64,"boundaries":[0_i64,5_i64]});
                path.extend(["actor", "chat", "turn", "key", "agent", "owner", "v1"]);
                marker = "encryption_version";
            }
            Adapter::GeneratedArtifact => {
                row.extend(doc!{"watch_id":"watch","operation_id":"op","source_result_identity":"source","artifact_index":0,"filename":"file","source_path_sha256":"path","content_type":"text/plain","size":5_i64,"sha256":hex::encode(Sha256::digest(&plain)),"preview_kind":"text","assistant_attachment_id":"attachment","purge_after":now});
                path.extend([
                    "conversation",
                    "chat",
                    "turn",
                    "turn",
                    "watch",
                    "watch",
                    "operation",
                    "op",
                    "source",
                    "source",
                ]);
                absent.insert("state", "expired");
            }
            Adapter::Public if f.collection() == "platform_public_sessions" => {
                row.extend(doc!{"profile_id":"profile","manager_bot_id":"99","revoked_at":Bson::Null,"telegram_bot_id":99_i64,"sender_id":5_i64,"chat_id":5_i64,"manager_authority_epoch":1,"public_policy_revision":1,"policy_digest":"digest","nyxagent_profile":"profile","credential_api_key_id":"key","upstream_may_exist":true,"effect_revision":0,"cleanup_complete":false,"sponsor_owner_user_id":"owner"});
                path.extend(["api_key", "key"]);
                if label == "upstream_binding_ciphertext" {
                    plain = br#"{"session_id":"remote"}"#.to_vec();
                }
                absent.extend(
                    doc! {"revoked_at":now,"upstream_may_exist":false,label:bin(Vec::new())},
                );
            }
            Adapter::Public => {
                row.extend(doc!{"public_session_id":"session","receipt_id":"receipt","claim_id":"claim","policy_digest":"digest","credential_api_key_id":"key","max_output_bytes":1024,"sponsor_owner_user_id":"owner"});
                path.extend([
                    "session",
                    "session",
                    "conversation",
                    "chat",
                    "receipt",
                    "receipt",
                    "claim",
                    "claim",
                ]);
                absent.insert(label, bin(Vec::new()));
            }
            Adapter::AsyncResult => {
                row.extend(doc!{"state":"ready","service_id":"service","submit_endpoint_id":"submit","contract":{"status_operation":"status","result_operation":"result","id_field":"/id","id_parameter":"id","status_field":"/status","success_states":["done"],"failure_states":["failed"]},"delivery":{"origin":"user"},"attempts":0,"lease_until":Bson::Null,"deadline":now,"poll_at":now});
                absent.insert("state", "delivered");
            }
            Adapter::CmaRequest => {
                plain = br#"{"name":"example"}"#.to_vec();
                row.insert(
                    "request_hash",
                    crate::services::assistant_cma_sandbox_service::prepare_creation_request(
                        &plain,
                    )
                    .unwrap()
                    .hash,
                );
                absent.extend(doc!{"submission_state":"confirmed_not_created","occupies_account_slot":false,"admission_state":"retired"});
                marker = "request_context_version";
            }
            Adapter::Capability => {
                plain=br#"{"byo":false,"unavailable_family":"unattributed_oauth","access_token":"value"}"#.to_vec();
                absent.insert("status", "complete");
                marker = "context_version";
                if !current {
                    row.extend(doc!{"context_version":2,label:bin(HistoricalAad::erasure_capability("row","owner").seal(k,&plain).await.unwrap())});
                    continue;
                }
            }
            _ => unreachable!(),
        }
        row.insert(
            label,
            bin(seal(k, f.collection(), "row", "owner", &path, &plain).await),
        );
        if current {
            if marker == "secret_context_versions" {
                versions.insert(label, 3);
            } else {
                row.insert(marker, 3);
            }
        }
    }
    if !versions.is_empty() {
        row.insert("secret_context_versions", versions);
    }
    let mut cleared = row.clone();
    for &label in f.entry.fields {
        cleared.remove(label);
    }
    for marker in [
        "secret_context_versions",
        "encryption_version",
        "request_context_version",
    ] {
        cleared.remove(marker);
    }
    cleared.extend(absent);
    (row, cleared)
}

pub(super) async fn check(
    k: &EncryptionKeys,
    f: &Family,
    row: &Document,
    parents: &Parents,
) -> Observation {
    let before = bson::to_vec(row).unwrap();
    let stats = k.decrypt_stats();
    let out = f.observe(k, row, parents).await;
    assert_eq!(before, bson::to_vec(row).unwrap());
    assert_eq!(stats, k.decrypt_stats());
    out
}

#[tokio::test]
async fn retained_storage_registry_matrix() {
    let mut reader = test_encryption_keys();
    reader.set_allow_legacy_record_reads(false);
    let parents = parent_rows();
    let mut seen = BTreeSet::new();
    for f in families().filter(|f| super::super::retained::source_companions(f.entry).is_some()) {
        seen.insert(f.collection());
        for current in [true, false] {
            let mut writer = test_encryption_keys();
            writer.set_contextual_record_writes(current);
            let (row, absent) = retained_fixture(&f, &writer, current).await;
            let result = check(&reader, &f, &row, &parents).await;
            let expected = f
                .entry
                .fields
                .iter()
                .filter(|k| row.contains_key(**k))
                .count();
            assert_eq!(materials(&result).count(), expected, "{}", f.collection());
            assert!(
                result
                    .findings
                    .iter()
                    .all(|(_, r)| !current && *r == Finding::AmbiguousBinding),
                "{} {:?}",
                f.collection(),
                result.findings
            );
            let cleared = check(&reader, &f, &absent, &parents).await;
            assert_eq!(materials(&cleared).count(), 0);
            let required = matches!(
                f.entry.adapter,
                Adapter::AssistantCredential | Adapter::Attachment | Adapter::ToolResult
            ) || f.collection() == "platform_public_turns";
            assert_eq!(!cleared.findings.is_empty(), required, "{}", f.collection());
            for &field in f.entry.fields {
                if !row.contains_key(field) {
                    continue;
                }
                let mut bad = row.clone();
                bad.insert(field, bin(vec![0]));
                let partial = check(&reader, &f, &bad, &parents).await;
                assert!(!partial.findings.is_empty(), "{}", f.collection());
                assert_eq!(
                    materials(&partial).count(),
                    expected - 1,
                    "{}",
                    f.collection()
                );
            }
            if current && matches!(f.entry.adapter, Adapter::Capability) {
                let mut implicit = row.clone();
                implicit.remove("context_version");
                assert!(matches!(
                    materials(&check(&reader, &f, &implicit, &parents).await).next(),
                    Some((_, Marker::ImplicitV3, _))
                ));
                implicit.insert("context_version", 3_i64);
                assert_eq!(
                    materials(&check(&reader, &f, &implicit, &parents).await).count(),
                    0
                );
            }
        }
    }
    assert_eq!(seen.len(), 10);
}

#[tokio::test]
async fn missing_root_edge_resolves_at_a_later_cut_for_chunk_and_page() {
    use crate::services::encrypted_record_census::{Census, Scope};
    let db = crate::test_utils::connect_transaction_test_database("observed_attachment_cut").await;
    crate::services::encrypted_record_census::tests::with_history(&db, 120, async {

    let f = family("assistant_attachments", "text_encrypted");
    let k = test_encryption_keys();
    let collection = db.collection::<Document>(f.collection());
    let now = DateTime::now();
    let mut children = Vec::new();
    for (id, path) in [
        (
            "chunk",
            vec![
                "data_encrypted",
                "parent_attachment_id",
                "root",
                "chunk_index",
                "0",
            ],
        ),
        (
            "page",
            vec![
                "data_encrypted",
                "parent_attachment_id",
                "root",
                "page",
                "1",
                "render_version",
                "poppler-jpeg-2048-v1",
            ],
        ),
    ] {
        let mut row = doc! {"_id":id,"user_id":"owner","parent_attachment_id":"root","group_id":Bson::Null,"conversation_id":"chat","data_encrypted":bin(seal(&k,f.collection(),id,"owner",&path,b"hello").await),"secret_context_versions":{"data_encrypted":3},"chunk_index":0_i64};
        if id == "page" {
            row.extend(doc!{"origin":"pdf_page","page":1,"render_version":"poppler-jpeg-2048-v1","content_type":"image/jpeg","size":5_i64,"chunk_index":"pdf/index","created_at":now});
        }
        children.push(row);
    }
    collection.insert_many(&children).await.unwrap();
    let scope = Scope {
        registry_digest: "fixture".into(),
        restore_epoch: "original".into(),
        deployment_revision: "one".into(),
        namespaces: [f.collection().to_owned()].into(),
    };
    let (census, start) = Census::start(&db, &k, scope).await.unwrap();
    let at = census.snapshot(&start).await.unwrap();
    for row in &children {
        let out = f.observe_at(&k, row, &at).await.unwrap();
        assert!(
            out.findings
                .contains(&("data_encrypted", Finding::MissingParent))
        );
        assert_eq!(materials(&out).count(), 1);
        assert_eq!(out.parents.len(), 1);
        assert_eq!(out.parents[0].id.as_str(), Some("root"));
    }
    collection.insert_one(doc!{"_id":"root","user_id":"owner","conversation_id":"chat","origin":"user_upload","label":"file","content_type":"application/pdf","size":5_i64,"chunks":1,"pages":1,"text_encrypted":bin(vec![]),"created_at":now}).await.unwrap();
    census.request_cut().await.unwrap();
    let end = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            if let Some(cut) = census.capture().await.unwrap() {
                break cut;
            }
        }
    })
    .await
    .unwrap();
    let at = census.snapshot(&end).await.unwrap();
    for row in &children {
        let out = f.observe_at(&k, row, &at).await.unwrap();
        assert!(out.findings.is_empty(), "{:?}", out.findings);
        assert_eq!(materials(&out).count(), 1);
        let mut wrong = row.clone();
        wrong.insert("group_id", "foreign");
        assert!(
            !f.observe_at(&k, &wrong, &at)
                .await
                .unwrap()
                .findings
                .is_empty()
        );
    }
    }).await;
    db.drop().await.unwrap();
}

#[tokio::test]
async fn image_attachment_uses_origin_and_generated_lineage() {
    use crate::{
        models::assistant_attachment::AssistantAttachment, services::assistant_record_service,
    };
    let f = family("assistant_attachments", "text_encrypted");
    let reader = test_encryption_keys();
    for origin in ["tool", "machine_preview", "generated_artifact"] {
        for current in [true, false] {
            let mut writer = test_encryption_keys();
            writer.set_contextual_record_writes(current);
            let mut row = doc! {"_id":"attachment","user_id":"owner","turn_id":"turn","conversation_id":"chat","origin":origin,
            "content_type":"text/plain","size":5_i64,"created_at":DateTime::now(),"data_encrypted":bin(Vec::new()),
            "source_turn_id":"source-turn","source_watch_id":"watch","source_operation_id":"operation","source_result_identity":"result"};
            let stored: AssistantAttachment = bson::from_document(row.clone()).unwrap();
            let bytes = assistant_record_service::encrypt_image(&writer, &stored, b"hello")
                .await
                .unwrap();
            row.insert("data_encrypted", bson::to_bson(&bytes).unwrap());
            if current {
                row.insert("secret_context_versions", doc! {"data_encrypted":3});
            }
            let out = check(&reader, &f, &row, &Default::default()).await;
            assert_eq!(materials(&out).count(), 1);
            assert_eq!(out.findings.is_empty(), current);
            if current && origin == "generated_artifact" {
                row.insert("source_result_identity", "successor");
                let swapped = check(&reader, &f, &row, &Default::default()).await;
                assert!(
                    swapped
                        .findings
                        .contains(&("data_encrypted", Finding::Authentication))
                );
            }
        }
    }
}

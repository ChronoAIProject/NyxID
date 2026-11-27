use super::{retained::check, *};
use crate::crypto::record_field::historical::{HistoricalAad, ProposalDomain};

fn proposal_arm() -> Document {
    let audience = doc! {"kind":"private","user_id":"owner"};
    doc! {"revision":1,"gap_key":"gap","effect_id":"effect","prepared_digest":"prepared","authority_digest":"authority","key_id":"key","conversation_id":"chat","source_turn_id":"turn","state":"pending","choices":[],
    "task":{"version":1,"task":{"root_kind":"message","root_id":"root"},"actor_user_id":"owner","audience":&audience,"epoch":1,"hard_deadline":DateTime::now(),"cancelled":false},
    "contract":{"revision":1,"subject":{"actor_user_id":"owner","principal":{"kind":"agent","id":"agent"},"lane":"owner"},"binding":{"connection_id":"connection","owner_user_id":"owner","credential_epoch":1,"authority_digest":"authority"},"context":{"conversation":{"kind":"same_audience"},"audience":audience},"operations":{"revision":1,"operations":[]},"validity":{"kind":"persistent"},"satisfies":[]},
    "review":{"agent":"agent","connection":"connection","operation":"operation","audience":"audience","scope":"scope"},
    "source":{"kind":"external"},"locator":"locator","source_digest":"digest"}
}

async fn approvals(f: &Family, k: &EncryptionKeys, current: bool) {
    let parents = Default::default();
    let mut mixed = doc! {"version":1};
    let mut paths = BTreeSet::new();
    for (i, slot) in ["v1", "v2", "v3", "v4"].iter().copied().enumerate() {
        let mut arm = proposal_arm();
        for name in ["payload", "result"] {
            paths.insert(format!("execution_proposal.{slot}.{name}"));
            let effect = serde_json::json!({"connection":"connection","catalog":"catalog","method":"GET","path":"/","body":"","headers":[]});
            let plain = match (slot, name) {
                ("v2", "payload") => serde_json::to_vec(
                    &serde_json::json!({"adapter":{"kind":"http"},"effect":effect}),
                )
                .unwrap(),
                ("v2", "result") => serde_json::to_vec(
                    &serde_json::json!({"status":200,"headers":[],"body":"ok","partial":false}),
                )
                .unwrap(),
                (_, "payload") => serde_json::to_vec(&effect).unwrap(),
                _ => b"ok".to_vec(),
            };
            let bytes = if current {
                arm.insert(format!("{name}_context_version"), 3);
                seal(
                    k,
                    f.collection(),
                    "row",
                    "owner",
                    &["execution_proposal", slot, name],
                    &plain,
                )
                .await
            } else {
                let domain = if slot == "v2" {
                    ProposalDomain::External
                } else {
                    ProposalDomain::Assistant
                };
                let aad = if name == "payload" {
                    HistoricalAad::proposal_payload(domain, "row")
                } else {
                    HistoricalAad::proposal_result(domain, "row")
                };
                aad.seal(k, &plain).await.unwrap()
            };
            arm.insert(name, bin(bytes));
        }
        let row = doc! {"_id":"row","user_id":"owner","execution_proposal":{"version":i as i32+1,slot:&arm}};
        let out = check(k, f, &row, &parents).await;
        assert!(out.findings.is_empty(), "{slot} {:?}", out.findings);
        assert_eq!(materials(&out).count(), 2);
        mixed.insert(slot, arm);
    }
    assert_eq!(paths, f.fields().map(str::to_owned).collect());
    let mut row = doc! {"_id":"row","user_id":"owner","execution_proposal":mixed};
    let out = check(k, f, &row, &parents).await;
    assert_eq!(materials(&out).count(), 8);
    assert!(!out.findings.is_empty());
    row.get_document_mut("execution_proposal")
        .unwrap()
        .get_document_mut("v4")
        .unwrap()
        .insert("payload", "broken");
    let out = check(k, f, &row, &parents).await;
    assert_eq!(materials(&out).count(), 7);
    assert!(!out.findings.is_empty());
    row.remove("execution_proposal");
    let empty = check(k, f, &row, &parents).await;
    assert_eq!(empty.instances.len(), 8);
    assert!(empty.findings.is_empty());
}

async fn learning(f: &Family, k: EncryptionKeys, current: bool) {
    use crate::services::assistant_skill_authoring::tests::recovery::mixed_authored_row;
    let mut reader = test_encryption_keys();
    reader.set_allow_legacy_record_reads(false);
    let bundle = Box::pin(mixed_authored_row(Arc::new(k))).await;
    let row = bundle.get_document("authored").unwrap();
    let parents = Default::default();
    let out = check(&reader, f, row, &parents).await;
    assert_eq!(materials(&out).count(), 3);
    assert!(
        out.findings
            .iter()
            .all(|(_, r)| !current && *r == Finding::AmbiguousBinding)
    );
    let mut bad = row.clone();
    bad.insert("body_encrypted", "wrong BSON");
    let out = check(&reader, f, &bad, &parents).await;
    assert_eq!(materials(&out).count(), 2);
    assert!(!out.findings.is_empty());
    let mut bad = row.clone();
    bad.get_document_mut("publication")
        .unwrap()
        .insert("sha256", "wrong package");
    let out = check(&reader, f, &bad, &parents).await;
    assert_eq!(materials(&out).count(), 3);
    assert!(
        out.findings
            .contains(&("publication.interface_encrypted", Finding::InvalidValue))
    );
    let mut cleared = row.clone();
    for field in [
        "body_encrypted",
        "scope_encrypted",
        "body_context_version",
        "scope_context_version",
        "scope_digest",
        "publication",
    ] {
        cleared.remove(field);
    }
    cleared.extend(doc! {"body_bytes":0_i64,"status":"rejected"});
    let out = check(&reader, f, &cleared, &parents).await;
    assert!(out.findings.is_empty());
    assert_eq!(materials(&out).count(), 0);
}

async fn window(f: &Family, k: &EncryptionKeys, current: bool) {
    let plain=br#"{"body":{},"prefix":"test","binding":{"session_id":"session","fingerprint":"fingerprint","guest":false}}"#;
    let mut w = doc! {"key":"window","capacity_deadline":DateTime::from_millis(0)};
    let bytes = if current {
        w.insert("input_context_version", 3);
        seal(
            k,
            f.collection(),
            "row",
            "owner",
            &[
                "active_turn",
                "restart_admission",
                "window",
                "input",
                "admitted",
                "window",
            ],
            plain,
        )
        .await
    } else {
        HistoricalAad::reattach_window("owner", "row", "admitted", "window")
            .unwrap()
            .seal(k, plain)
            .await
            .unwrap()
    };
    w.insert("input", bin(bytes));
    let mut row = doc! {"_id":"row","user_id":"owner","credential_api_key_id":"successor","active_turn":{"restart_admission":{"credential_id":"admitted","credential_ended":true,"window":w}}};
    let parents = Default::default();
    let out = check(k, f, &row, &parents).await;
    assert!(out.findings.is_empty());
    assert_eq!(materials(&out).count(), 1);
    row.get_document_mut("active_turn")
        .unwrap()
        .get_document_mut("restart_admission")
        .unwrap()
        .insert("credential_ended", false);
    assert!(!check(k, f, &row, &parents).await.findings.is_empty());
    row.remove("active_turn");
    assert!(matches!(
        check(k, f, &row, &parents).await.instances.as_slice(),
        [(_, Storage::Absent)]
    ));
}

#[tokio::test]
async fn nested_registry_observes_retained_arms_and_validates_publication() {
    let mut seen = BTreeSet::new();
    for f in families().filter(|f| super::super::nested::source_companions(f.entry).is_some()) {
        seen.insert(f.collection());
        for current in [false, true] {
            let mut k = test_encryption_keys();
            k.set_contextual_record_writes(current);
            k.set_allow_legacy_record_reads(false);
            match f.entry.adapter {
                Adapter::ReattachWindow => window(&f, &k, current).await,
                Adapter::ApprovalProposal => approvals(&f, &k, current).await,
                Adapter::LearningProposal => {
                    // The legacy fixture's serving writer also reads back its own work.
                    k.set_allow_legacy_record_reads(true);
                    Box::pin(learning(&f, k, current)).await;
                }
                _ => unreachable!(),
            }
        }
    }
    assert_eq!(seen.len(), 3);
}

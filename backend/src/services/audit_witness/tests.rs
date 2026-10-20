use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf, process::Command, sync::Arc};

use ed25519_dalek::VerifyingKey;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

use super::*;

struct Fixture {
    _dir: TempDir,
    admin: PathBuf,
    journal: PathBuf,
    enrollment: Enrollment,
}
impl Fixture {
    fn new() -> Self {
        Self::with_head(AuditLink {
            key_version: 1,
            seq: 0,
            prev_hash: ZERO_HASH.into(),
            entry_hash: ZERO_HASH.into(),
        })
    }
    fn with_head(head: AuditLink) -> Self {
        let base = fs::canonicalize(std::env::temp_dir()).unwrap();
        let dir = tempfile::tempdir_in(base).unwrap();
        let admin = dir.path().join("admin");
        let journal = dir.path().join("journal");
        let enrollment = storage::fixture_create(&admin, &journal, Uuid::new_v4(), head).unwrap();
        Self {
            _dir: dir,
            admin,
            journal,
            enrollment,
        }
    }
    fn open(&self) -> ReferenceWitness {
        let (e, k, j) = storage::fixture_open(&self.admin, &self.journal).unwrap();
        ReferenceWitness::recover(e, k, j).unwrap()
    }
    fn key(&self) -> VerifyingKey {
        VerifyingKey::from_bytes(
            &hex::decode(&self.enrollment.verification_key)
                .unwrap()
                .try_into()
                .unwrap(),
        )
        .unwrap()
    }
    fn observe(&self, witness: &ReferenceWitness) -> SignedObservation {
        witness
            .observe(ObserveRequest {
                chain_id: self.enrollment.chain_id,
                challenge: "c".repeat(64),
            })
            .unwrap()
    }
    fn request(&self, witness: &ReferenceWitness) -> AdvanceRequest {
        next_request(&self.observe(witness).body.latest, Uuid::new_v4())
    }
}
fn next_request(receipt: &SignedReceipt, request_id: Uuid) -> AdvanceRequest {
    let seq = receipt.body.head.seq + 1;
    AdvanceRequest {
        chain_id: receipt.body.chain_id,
        request_id,
        previous_receipt: receipt_digest(receipt),
        links: vec![AuditLink {
            key_version: receipt.body.head.key_version,
            seq,
            prev_hash: receipt.body.head.entry_hash.clone(),
            entry_hash: hex::encode(Sha256::digest(seq.to_be_bytes())),
        }],
    }
}

#[test]
fn advance_survives_restart_and_identical_retry() {
    let f = Fixture::new();
    let w = f.open();
    let request = f.request(&w);
    let receipt = w.advance(request.clone()).unwrap();
    assert_eq!(receipt.body.head.seq, 1);
    assert_eq!(receipt.body.head.key_version, 1);
    verify_receipt(&receipt, f.enrollment.chain_id, &f.key()).unwrap();
    drop(w);
    let reopened = f.open();
    assert_eq!(reopened.advance(request).unwrap(), receipt);
    assert_eq!(f.observe(&reopened).body.latest, receipt);
    assert_eq!(
        reopened
            .history(HistoryRequest {
                chain_id: f.enrollment.chain_id,
                from_receipt: 0,
                limit: 128
            })
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn adoption_binds_existing_v1_tail_without_resigning_it() {
    let head = AuditLink {
        key_version: 9,
        seq: 400,
        prev_hash: "1".repeat(64),
        entry_hash: "2".repeat(64),
    };
    let f = Fixture::with_head(head.clone());
    let w = f.open();
    assert_eq!(f.observe(&w).body.latest.body.head, head);
    let receipt = w.advance(f.request(&w)).unwrap();
    assert_eq!(receipt.body.head.seq, 401);
    assert_eq!(receipt.body.head.prev_hash, head.entry_hash);
}

#[test]
fn rejects_forks_gaps_nonmonotonic_versions_chains_and_conflicting_retries() {
    let f = Fixture::new();
    let w = f.open();
    let base = f.request(&w);
    let mut invalid = Vec::new();
    let mut r = base.clone();
    r.links[0].seq = 0;
    invalid.push(r);
    let mut r = base.clone();
    r.links[0].seq = 2;
    invalid.push(r);
    let mut r = base.clone();
    r.links[0].prev_hash = "f".repeat(64);
    invalid.push(r);
    let mut r = base.clone();
    r.links[0].entry_hash = "A".repeat(64);
    invalid.push(r);
    let mut r = base.clone();
    r.links[0].key_version = 2;
    invalid.push(r);
    let mut r = base.clone();
    r.chain_id = Uuid::new_v4();
    invalid.push(r);
    let mut r = base.clone();
    r.previous_receipt = ZERO_HASH.into();
    invalid.push(r);
    let mut r = base.clone();
    r.request_id = Uuid::nil();
    invalid.push(r);
    let mut r = base.clone();
    r.links.clear();
    invalid.push(r);
    let mut r = base.clone();
    r.links = vec![base.links[0].clone(); MAX_LINKS + 1];
    invalid.push(r);
    for r in invalid {
        assert!(w.advance(r).is_err());
    }
    let receipt = w.advance(base.clone()).unwrap();
    let mut r = base.clone();
    r.links[0].entry_hash = "e".repeat(64);
    assert_eq!(w.advance(r).unwrap_err(), WitnessError::Conflict);
    let mut r = base.clone();
    r.request_id = Uuid::new_v4();
    assert_eq!(w.advance(r).unwrap_err(), WitnessError::Conflict);
    assert_eq!(f.observe(&w).body.latest, receipt);
}

#[test]
fn checks_every_middle_link_in_a_batch() {
    let f = Fixture::new();
    let w = f.open();
    let mut r = f.request(&w);
    for seq in 2..=MAX_LINKS as u64 {
        r.links.push(AuditLink {
            key_version: 1,
            seq,
            prev_hash: r.links.last().unwrap().entry_hash.clone(),
            entry_hash: hex::encode(Sha256::digest(seq.to_be_bytes())),
        });
    }
    let mut fork = r.clone();
    fork.links[63].prev_hash = "f".repeat(64);
    assert!(w.advance(fork).is_err());
    assert_eq!(w.advance(r).unwrap().body.head.seq, 128);
}

#[test]
fn concurrent_same_predecessor_has_one_winner() {
    let f = Fixture::new();
    let w = Arc::new(f.open());
    let request = f.request(&w);
    let barrier = Arc::new(std::sync::Barrier::new(8));
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let w = w.clone();
            let mut r = request.clone();
            r.request_id = Uuid::new_v4();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                w.advance(r).is_ok()
            })
        })
        .collect();
    assert_eq!(
        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .filter(|v| *v)
            .count(),
        1
    );
    assert_eq!(f.observe(&w).body.latest.body.receipt_seq, 1);
}

#[test]
fn rollback_then_new_writes_cannot_skip_the_previously_witnessed_fork() {
    let f = Fixture::new();
    let w = f.open();
    let saved = f.request(&w);
    let receipt = w.advance(saved.clone()).unwrap();
    // A restored Mongo can produce a higher sequence, but it must supply all
    // links from the protected predecessor, not just a larger final head.
    let mut forged = next_request(&receipt, Uuid::new_v4());
    forged.previous_receipt = saved.previous_receipt;
    forged.links[0].seq = 100;
    forged.links[0].prev_hash = "e".repeat(64);
    assert_eq!(w.advance(forged).unwrap_err(), WitnessError::Conflict);
    assert_eq!(f.observe(&w).body.latest, receipt);
}

#[test]
fn rewritten_historical_hashes_cannot_replace_witnessed_history() {
    let f = Fixture::new();
    let w = f.open();
    let old_head = f.observe(&w).body.latest;
    w.advance(f.request(&w)).unwrap();
    // The app can recompute its HMAC rows, but cannot replace the sink's
    // predecessor or produce its independent Ed25519 receipt signature.
    let mut rewritten = next_request(&old_head, Uuid::new_v4());
    rewritten.links[0].entry_hash = "a".repeat(64);
    assert_eq!(w.advance(rewritten).unwrap_err(), WitnessError::Conflict);
    let mut receipt = f.observe(&w).body.latest;
    receipt.body.head.entry_hash = "a".repeat(64);
    assert!(verify_receipt(&receipt, f.enrollment.chain_id, &f.key()).is_err());
}

#[test]
fn dispatch_claim_is_not_reassignable_after_restore_restart_or_expiry() {
    let f = Fixture::new();
    let w = f.open();
    let intent = w.advance(f.request(&w)).unwrap();
    let claim = DispatchRequest {
        chain_id: f.enrollment.chain_id,
        operation_id: Uuid::new_v4(),
        attempt_id: Uuid::new_v4(),
        process_id: Uuid::new_v4(),
        intent_seq: intent.body.head.seq,
        intent_hash: intent.body.head.entry_hash.clone(),
        previous_receipt: receipt_digest(&intent),
    };
    let receipt = w.claim_dispatch(claim.clone()).unwrap();
    drop(w);
    let w = f.open();
    assert_eq!(w.claim_dispatch(claim.clone()).unwrap(), receipt);
    for change_process in [false, true] {
        let mut retry = claim.clone();
        retry.previous_receipt = receipt_digest(&receipt);
        if change_process {
            retry.process_id = Uuid::new_v4();
        } else {
            retry.attempt_id = Uuid::new_v4();
        }
        assert_eq!(w.claim_dispatch(retry).unwrap_err(), WitnessError::Conflict);
    }
    // There is deliberately no timeout/lease field permitting re-dispatch.
    assert_eq!(f.observe(&w).body.latest, receipt);
}

#[test]
fn dispatch_requires_an_exact_witnessed_non_genesis_intent() {
    let f = Fixture::new();
    let w = f.open();
    let r = f.observe(&w).body.latest;
    let claim = DispatchRequest {
        chain_id: f.enrollment.chain_id,
        operation_id: Uuid::new_v4(),
        attempt_id: Uuid::new_v4(),
        process_id: Uuid::new_v4(),
        intent_seq: 0,
        intent_hash: ZERO_HASH.into(),
        previous_receipt: receipt_digest(&r),
    };
    assert!(w.claim_dispatch(claim.clone()).is_err());
    let mut c = claim;
    c.intent_seq = 1;
    c.intent_hash = "e".repeat(64);
    assert!(w.claim_dispatch(c).is_err());
}

#[test]
fn quiet_chain_requires_a_fresh_challenge_and_bounded_signed_time() {
    let f = Fixture::new();
    let w = f.open();
    let observation = f.observe(&w);
    let time = observation.body.witness_time_ms;
    verify_observation(
        &observation,
        f.enrollment.chain_id,
        &f.key(),
        &"c".repeat(64),
        time,
        time,
    )
    .unwrap();
    assert!(
        verify_observation(
            &observation,
            f.enrollment.chain_id,
            &f.key(),
            &"d".repeat(64),
            time,
            time
        )
        .is_err()
    );
    assert!(
        verify_observation(
            &observation,
            f.enrollment.chain_id,
            &f.key(),
            &"c".repeat(64),
            time + 1,
            time + 2
        )
        .is_err()
    );
    assert!(
        verify_observation(
            &observation,
            f.enrollment.chain_id,
            &f.key(),
            &"c".repeat(64),
            time - 2,
            time - 1
        )
        .is_err()
    );
    let mut forged = observation.clone();
    forged.body.witness_time_ms += 1;
    assert!(
        verify_observation(
            &forged,
            f.enrollment.chain_id,
            &f.key(),
            &"c".repeat(64),
            time,
            time + 2
        )
        .is_err()
    );
    assert_eq!(f.observe(&w).body.latest, observation.body.latest);
}

#[test]
fn another_provider_can_complete_an_authenticated_process_local_challenge() {
    let f = Fixture::new();
    let w = f.open();
    let challenge = ObservationChallenge::begin(f.enrollment.chain_id).unwrap();
    let observed = w.observe(challenge.request().clone()).unwrap();
    let contact = challenge.complete(observed, &f.key()).unwrap();
    assert_eq!(contact.receipt().body.head.seq, 0);
    assert!(contact.age().as_secs() < 1);
    let challenge = ObservationChallenge::begin(f.enrollment.chain_id).unwrap();
    // A replay from another exchange cannot complete this fresh object.
    assert!(matches!(
        challenge.complete(f.observe(&w), &f.key()),
        Err(WitnessError::Corrupt)
    ));
}

#[test]
fn every_receipt_field_and_chain_identity_is_authenticated() {
    let f = Fixture::new();
    let w = f.open();
    let receipt = w.advance(f.request(&w)).unwrap();
    let mut changed = Vec::new();
    let mut r = receipt.clone();
    r.body.chain_id = Uuid::new_v4();
    changed.push(r);
    let mut r = receipt.clone();
    r.body.protocol += 1;
    changed.push(r);
    let mut r = receipt.clone();
    r.body.receipt_seq += 1;
    changed.push(r);
    let mut r = receipt.clone();
    r.body.previous_receipt = "e".repeat(64);
    changed.push(r);
    let mut r = receipt.clone();
    r.body.witness_time_ms += 1;
    changed.push(r);
    let mut r = receipt.clone();
    r.body.head.key_version += 1;
    changed.push(r);
    let mut r = receipt.clone();
    r.body.head.seq += 1;
    changed.push(r);
    let mut r = receipt.clone();
    r.body.head.prev_hash = "e".repeat(64);
    changed.push(r);
    let mut r = receipt.clone();
    r.body.head.entry_hash = "e".repeat(64);
    changed.push(r);
    let mut r = receipt.clone();
    if let WitnessEvent::Advance { request } = &mut r.body.event {
        request.request_id = Uuid::new_v4();
    }
    changed.push(r);
    let mut r = receipt.clone();
    r.signature = "00".repeat(64);
    changed.push(r);
    for r in changed {
        assert!(verify_receipt(&r, f.enrollment.chain_id, &f.key()).is_err());
    }
    assert!(verify_receipt(&receipt, Uuid::new_v4(), &f.key()).is_err());
    assert!(
        verify_receipt(
            &receipt,
            f.enrollment.chain_id,
            &SigningKey::from_bytes(&[19; 32]).verifying_key()
        )
        .is_err()
    );
}

#[test]
fn canonical_frames_distinguish_boundaries_and_domains() {
    let f = Fixture::new();
    let w = f.open();
    let receipt = w.advance(f.request(&w)).unwrap();
    let mut a = receipt.body.clone();
    let mut b = a.clone();
    a.head.prev_hash = "ab".into();
    a.head.entry_hash = "c".into();
    b.head.prev_hash = "a".into();
    b.head.entry_hash = "bc".into();
    assert_ne!(codec::receipt_bytes(&a), codec::receipt_bytes(&b));
    assert_ne!(
        codec::receipt_bytes(&receipt.body),
        codec::observation_bytes(&f.observe(&w).body)
    );
    let json = serde_json::to_value(&receipt).unwrap();
    let decoded: SignedReceipt = serde_json::from_value(json).unwrap();
    assert_eq!(receipt_digest(&decoded), receipt_digest(&receipt));
}

#[test]
fn replay_rejects_middle_deletion_and_modified_published_records() {
    for delete in [false, true] {
        let f = Fixture::new();
        let w = f.open();
        w.advance(f.request(&w)).unwrap();
        w.advance(f.request(&w)).unwrap();
        drop(w);
        let path = f.journal.join("00000000000000000001.json");
        if delete {
            fs::remove_file(path).unwrap();
        } else {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
            fs::write(&path, b"{}").unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
        }
        let (e, k, j) = storage::fixture_open(&f.admin, &f.journal).unwrap();
        assert!(ReferenceWitness::recover(e, k, j).is_err());
    }
}

#[test]
fn refuses_concurrent_writer_and_unsafe_same_uid_production_enrollment() {
    let f = Fixture::new();
    let _w = f.open();
    assert!(storage::fixture_open(&f.admin, &f.journal).is_err());
    assert!(ReferenceWitness::open(&f.admin, &f.journal).is_err());
}

#[test]
fn symlink_and_writable_enrollment_fail_without_permission_repair() {
    let f = Fixture::new();
    fs::set_permissions(
        f.admin.join("enrollment.json"),
        fs::Permissions::from_mode(0o660),
    )
    .unwrap();
    assert!(storage::fixture_open(&f.admin, &f.journal).is_err());
    assert_eq!(
        fs::metadata(f.admin.join("enrollment.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o660
    );
    fs::set_permissions(
        f.admin.join("enrollment.json"),
        fs::Permissions::from_mode(0o440),
    )
    .unwrap();
    fs::rename(f.admin.join("signing.key"), f.admin.join("other.key")).unwrap();
    std::os::unix::fs::symlink("other.key", f.admin.join("signing.key")).unwrap();
    assert!(storage::fixture_open(&f.admin, &f.journal).is_err());
}

#[test]
fn signing_seed_reader_rejects_short_and_oversized_files() {
    for length in [31, 33, 4096] {
        let f = Fixture::new();
        let path = f.admin.join("signing.key");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&path, vec![37_u8; length]).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o440)).unwrap();
        assert!(matches!(
            storage::fixture_open(&f.admin, &f.journal),
            Err(WitnessError::Corrupt)
        ));
    }
}

#[test]
fn journal_failure_poisoning_prevents_false_success() {
    let f = Fixture::new();
    let w = f.open();
    let request = f.request(&w);
    fs::set_permissions(&f.journal, fs::Permissions::from_mode(0o500)).unwrap();
    assert_eq!(w.advance(request).unwrap_err(), WitnessError::Storage);
    assert_eq!(
        w.observe(ObserveRequest {
            chain_id: f.enrollment.chain_id,
            challenge: "c".repeat(64)
        })
        .unwrap_err(),
        WitnessError::Unavailable
    );
    fs::set_permissions(&f.journal, fs::Permissions::from_mode(0o700)).unwrap();
    drop(w);
    assert_eq!(f.observe(&f.open()).body.latest.body.head.seq, 0);
}

#[test]
fn crash_child() {
    let Ok(dir) = std::env::var("NYXID_WITNESS_TEST_DIR") else {
        return;
    };
    let (e, k, j) = storage::fixture_open(
        &PathBuf::from(&dir).join("admin"),
        &PathBuf::from(&dir).join("journal"),
    )
    .unwrap();
    let chain = e.chain_id;
    let w = ReferenceWitness::recover(e, k, j).unwrap();
    let head = w
        .observe(ObserveRequest {
            chain_id: chain,
            challenge: "c".repeat(64),
        })
        .unwrap()
        .body
        .latest;
    let request_id = std::env::var("NYXID_WITNESS_TEST_REQUEST")
        .unwrap()
        .parse()
        .unwrap();
    w.advance(next_request(&head, request_id)).unwrap();
    panic!("crash boundary was not reached");
}

#[test]
fn real_process_crashes_preserve_atomic_publication_and_retry() {
    for boundary in ["written", "file_synced", "published", "directory_synced"] {
        let f = Fixture::new();
        let w = f.open();
        let request = f.request(&w);
        drop(w);
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "services::audit_witness::tests::crash_child",
                "--nocapture",
            ])
            .env("NYXID_WITNESS_TEST_DIR", f._dir.path())
            .env("NYXID_WITNESS_TEST_REQUEST", request.request_id.to_string())
            .env("NYXID_WITNESS_TEST_CRASH", boundary)
            .output()
            .unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(output.status.signal(), Some(libc::SIGABRT), "{boundary}");
        let w = f.open();
        let before = f.observe(&w).body.latest.body.head.seq;
        assert_eq!(
            before,
            if ["published", "directory_synced"].contains(&boundary) {
                1
            } else {
                0
            }
        );
        let receipt = w.advance(request.clone()).unwrap();
        assert_eq!(receipt.body.head.seq, 1);
        assert_eq!(w.advance(request).unwrap(), receipt);
        assert_eq!(w.advance(f.request(&w)).unwrap().body.head.seq, 2);
    }
}

#[tokio::test]
async fn real_http_client_authentication_history_challenge_and_outage() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let f = Fixture::new();
    let w = Arc::new(f.open());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let endpoint = format!("http://{addr}");
    let server_w = w.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, crate::handlers::audit_witness::router(server_w))
            .await
            .unwrap()
    });
    let client = HttpWitness::loopback_for_validation(
        &endpoint,
        f.enrollment.chain_id,
        &f.enrollment.verification_key,
        Zeroizing::new("a".repeat(64)),
    )
    .unwrap();
    let provider: &dyn WitnessClient = &client;
    let first = provider.observe().await.unwrap();
    assert_eq!(first.receipt().body.head.seq, 0);
    assert!(first.age().as_secs() < 1);
    let request = next_request(first.receipt(), Uuid::new_v4());
    let receipt = provider.advance(request.clone()).await.unwrap();
    assert_eq!(provider.advance(request).await.unwrap(), receipt);
    assert_eq!(provider.history(0, 128).await.unwrap().len(), 2);
    assert_eq!(provider.observe().await.unwrap().receipt(), &receipt);
    let bad = HttpWitness::loopback_for_validation(
        &endpoint,
        f.enrollment.chain_id,
        &f.enrollment.verification_key,
        Zeroizing::new("b".repeat(64)),
    )
    .unwrap();
    assert!(matches!(
        bad.observe().await,
        Err(WitnessError::Unauthorized)
    ));
    let http = reqwest::Client::new();
    for path in ["/v1/enroll", "/v1/reset", "/v1/delete"] {
        assert_eq!(
            http.post(format!("{endpoint}{path}"))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }
    assert_eq!(
        http.delete(format!("{endpoint}/v1/advance"))
            .send()
            .await
            .unwrap()
            .status(),
        405
    );
    let request = ObserveRequest {
        chain_id: f.enrollment.chain_id,
        challenge: "c".repeat(64),
    };
    assert_eq!(
        http.post(format!("{endpoint}/v1/observe"))
            .header("authorization", format!("Bearer {}", "a".repeat(64)))
            .header("authorization", format!("Bearer {}", "a".repeat(64)))
            .json(&request)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    server.abort();
    let _ = server.await;
    assert!(matches!(
        provider.observe().await,
        Err(WitnessError::Unavailable)
    ));
}

#[test]
fn client_rejects_unsafe_transport_and_redacts_secrets_and_key_identifiers() {
    let f = Fixture::new();
    for endpoint in [
        "http://example.com",
        "http://127.0.0.1",
        "https://user:password@example.com",
        "https://example.com/?",
        "https://example.com/#",
        "https://example.com/prefix",
        "https://example.com:0",
    ] {
        assert!(
            HttpWitness::new(
                endpoint,
                f.enrollment.chain_id,
                &f.enrollment.verification_key,
                Zeroizing::new("a".repeat(64))
            )
            .is_err()
        );
    }
    assert!(
        HttpWitness::loopback_for_validation(
            "http://localhost:8091",
            f.enrollment.chain_id,
            &f.enrollment.verification_key,
            Zeroizing::new("a".repeat(64))
        )
        .is_err()
    );
    let client = HttpWitness::new(
        "https://witness.example",
        f.enrollment.chain_id,
        &f.enrollment.verification_key,
        Zeroizing::new("a".repeat(64)),
    )
    .unwrap();
    assert_eq!(format!("{client:?}"), "HttpWitness([REDACTED])");
}

#[tokio::test]
async fn http_client_rejects_valid_but_replayed_observation_and_wrong_pin() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let f = Fixture::new();
    let w = Arc::new(f.open());
    let replay = f.observe(&w);
    let mock = axum::Router::new().route(
        "/v1/observe",
        axum::routing::post(move || {
            let replay = replay.clone();
            async move { axum::Json(replay) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let client = HttpWitness::loopback_for_validation(
        &endpoint,
        f.enrollment.chain_id,
        &f.enrollment.verification_key,
        Zeroizing::new("a".repeat(64)),
    )
    .unwrap();
    assert!(matches!(client.observe().await, Err(WitnessError::Corrupt)));
    server.abort();
    let _ = server.await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, crate::handlers::audit_witness::router(w))
            .await
            .unwrap()
    });
    let wrong_pin = hex::encode(SigningKey::from_bytes(&[19; 32]).verifying_key().to_bytes());
    let client = HttpWitness::loopback_for_validation(
        &endpoint,
        f.enrollment.chain_id,
        &wrong_pin,
        Zeroizing::new("a".repeat(64)),
    )
    .unwrap();
    assert!(matches!(client.observe().await, Err(WitnessError::Corrupt)));
    server.abort();
    let _ = server.await;
}

#[test]
fn history_pagination_is_bounded_by_bytes_and_requires_explicit_continuation() {
    let f = Fixture::new();
    let w = f.open();
    for _ in 0..12 {
        let mut request = f.request(&w);
        for _ in 1..MAX_LINKS {
            let previous = request.links.last().unwrap();
            request.links.push(AuditLink {
                key_version: 1,
                seq: previous.seq + 1,
                prev_hash: previous.entry_hash.clone(),
                entry_hash: hex::encode(Sha256::digest(previous.entry_hash.as_bytes())),
            });
        }
        w.advance(request).unwrap();
    }
    let first = w
        .history(HistoryRequest {
            chain_id: f.enrollment.chain_id,
            from_receipt: 0,
            limit: MAX_LINKS,
        })
        .unwrap();
    assert!(first.len() < 13);
    assert!(serde_json::to_vec(&first).unwrap().len() <= MAX_HTTP_BYTES);
    let second = w
        .history(HistoryRequest {
            chain_id: f.enrollment.chain_id,
            from_receipt: first.len() as u64,
            limit: MAX_LINKS,
        })
        .unwrap();
    assert_eq!(first.len() + second.len(), 13);
    assert_eq!(
        second[0].body.previous_receipt,
        receipt_digest(first.last().unwrap())
    );
    assert!(
        w.history(HistoryRequest {
            chain_id: f.enrollment.chain_id,
            from_receipt: 0,
            limit: MAX_LINKS + 1
        })
        .is_err()
    );
}

#[tokio::test]
async fn http_adapter_refuses_oversized_bodies_and_unknown_fields() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let f = Fixture::new();
    let w = Arc::new(f.open());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, crate::handlers::audit_witness::router(w))
            .await
            .unwrap()
    });
    let http = reqwest::Client::new();
    let oversized = http
        .post(format!("{endpoint}/v1/observe"))
        .bearer_auth("a".repeat(64))
        .header("content-type", "application/json")
        .body(" ".repeat(MAX_HTTP_BYTES + 1))
        .send()
        .await
        .unwrap();
    assert_eq!(oversized.status(), 413);
    let invalid = http.post(format!("{endpoint}/v1/observe")).bearer_auth("a".repeat(64))
        .json(&serde_json::json!({"chain_id": f.enrollment.chain_id, "challenge": "c".repeat(64), "reset": true})).send().await.unwrap();
    assert_eq!(invalid.status(), 422);
    server.abort();
    let _ = server.await;
}

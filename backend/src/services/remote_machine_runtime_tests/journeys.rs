//! Composed public owner HTTP, authenticated managed WSS, bounded Manager and
//! one real BillingService. Only Lago, workload identity and the Kubernetes API/controller are fixtures.
use super::*;
use crate::services::{
    remote_machine_bootstrap::{BootstrapTicketVerifier, VerifiedBootstrapTicket},
    remote_machine_manager::{Manager, PlatformReadiness},
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use tower::ServiceExt;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
use zeroize::Zeroizing;

/// Local API/controller fixture: desired submission, application, suspension,
/// and resource garbage collection advance separately, as real observations do.
struct LocalKube {
    object: Mutex<Option<Value>>,
    apply_updates: AtomicBool,
    suspended: AtomicBool,
    cleanup: AtomicBool,
    delete_requested: AtomicBool,
    generation: AtomicUsize,
    sandbox_uid: String,
    pod_uid: Mutex<String>,
    pvc_uid: String,
    pv_uid: String,
    node_uid: String,
}
impl LocalKube {
    fn current(&self) -> Option<Value> {
        if self.delete_requested.load(Ordering::SeqCst) && self.cleanup.load(Ordering::SeqCst) {
            return None;
        }
        let mut value = self.object.lock().unwrap().clone()?;
        let generation = value["metadata"]["generation"].as_i64().unwrap();
        let requested = value["spec"]["operatingMode"] == "Suspended";
        let applied = requested && self.suspended.load(Ordering::SeqCst);
        value["status"]["conditions"] = json!([
            {"type":"Ready","status":if requested {"False"} else {"True"},
                "observedGeneration":generation,"reason":"FixtureControllerObserved"},
            {"type":"Suspended","status":if applied {"True"} else {"False"},
                "observedGeneration":generation,"reason":if applied {"PodTerminated"} else {"PodTerminating"}}
        ]);
        Some(value)
    }
    fn resource(&self, kind: &str) -> Option<Value> {
        let object = self.current()?;
        if kind == "Pod"
            && object["spec"]["operatingMode"] == "Suspended"
            && self.suspended.load(Ordering::SeqCst)
        {
            return None;
        }
        let (name, uid) = if kind == "Pod" {
            ("node", self.pod_uid.lock().unwrap().clone())
        } else {
            ("workspace", self.pvc_uid.clone())
        };
        let mut value = json!({"apiVersion":"v1","kind":kind,"metadata":{
            "name":name,"namespace":"nyxid-machine","uid":uid,"resourceVersion":"21",
            "labels":object["metadata"]["labels"],"ownerReferences":[{
                "apiVersion":"agents.x-k8s.io/v1beta1","kind":"Sandbox","name":object["metadata"]["name"],
                "uid":self.sandbox_uid,"controller":true
            }]},"spec":{},"status":{"phase":if kind=="Pod" {"Running"} else {"Bound"}}});
        if kind == "Pod" {
            value["spec"]["nodeName"] = json!("local-node");
            value["spec"]["volumes"] =
                json!([{"name":"workspace","persistentVolumeClaim":{"claimName":"workspace"}}]);
        } else {
            value["spec"]["volumeName"] = json!("local-pv");
        }
        Some(value)
    }
}
struct LocalIdentity {
    nonce: String,
    recipient: [u8; 32],
}
#[async_trait]
impl BootstrapTicketVerifier for LocalIdentity {
    async fn verify(
        &self,
        ticket: &Value,
        binding: &nyxid_machine::managed::Binding,
    ) -> AppResult<VerifiedBootstrapTicket> {
        if ticket["local_workload_nonce"].as_str() != Some(self.nonce.as_str()) {
            return Err(AppError::Unauthorized(
                "local workload identity denied".into(),
            ));
        }
        Ok(VerifiedBootstrapTicket::from_provider_verified_claims(
            binding.clone(),
            self.recipient,
            Utc::now() + Duration::minutes(5),
        ))
    }
}
struct Journey {
    f: Fixture,
    app: Router,
    state: crate::AppState,
    owner_token: Zeroizing<String>,
    address: std::net::SocketAddr,
    server: tokio::task::JoinHandle<()>,
    _sandbox: MockServer,
    manager: Manager,
    kube: Arc<LocalKube>,
    recipient: nyxid_machine::bootstrap::RecipientKey,
    nonce: String,
}

struct ManagedSocket {
    task: tokio::task::JoinHandle<()>,
    shutdown: tokio::sync::oneshot::Sender<()>,
}
impl ManagedSocket {
    async fn close(self) {
        let _ = self.shutdown.send(());
        tokio::time::timeout(std::time::Duration::from_secs(5), self.task)
            .await
            .expect("bounded managed fixture socket shutdown")
            .expect("managed fixture validates every received runtime window");
    }
}
impl Drop for Journey {
    fn drop(&mut self) {
        self.server.abort();
    }
}
async fn http(
    j: &Journey,
    method: &str,
    path: &str,
    body: Value,
    key: Option<&str>,
    human: bool,
) -> (StatusCode, Value) {
    let mut r = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if human {
        r = r.header(
            "authorization",
            format!("Bearer {}", j.owner_token.as_str()),
        );
    }
    if let Some(key) = key {
        r = r.header("idempotency-key", key);
    }
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        j.app
            .clone()
            .oneshot(r.body(Body::from(body.to_string())).unwrap()),
    )
    .await
    .expect("bounded owner HTTP")
    .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
async fn fixture_http(label: &str) -> Journey {
    let f = fixture(label).await;
    personal_remote_flag(&f, true).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut config = crate::test_utils::test_app_config();
    config.billing_enabled = true;
    config.base_url = format!("http://{address}");
    let mut state = crate::test_utils::test_app_state_with_config(f.db.clone(), config);
    state.billing = f.billing.clone();
    let recipient = nyxid_machine::bootstrap::generate_recipient().unwrap();
    let nonce = Uuid::new_v4().to_string();
    state.managed_bootstrap_verifier = Arc::new(LocalIdentity {
        nonce: nonce.clone(),
        recipient: nyxid_machine::bootstrap::recipient_key_digest(
            &recipient.public_key_b64u().unwrap(),
        )
        .unwrap(),
    });
    let profile = lifecycle::fixed_profile(format!("registry.test/node@sha256:{}", "a".repeat(64)));
    state.local_machine_platform = Some(Arc::new(crate::test_utils::LocalMachinePlatform {
        profile,
        readiness: PlatformReadiness {
            configuration_ready: true,
            manager_started: true,
            namespace: "nyxid-machine",
            blockers: vec![],
            physical_acceptance: "native_state_only_no_physical_guarantee",
            lifecycle_basis: "agent_sandbox_native_observation",
        },
    }));
    let owner_token = Zeroizing::new(
        crate::crypto::jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &Uuid::parse_str(&f.owner).unwrap(),
            "read write",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap(),
    );
    let (a, b) = crate::routes::build_router_with_state(state.clone());
    let app = a.merge(b).with_state(state.clone());
    let served = app.clone();
    let server = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            served.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await;
    });
    let sandbox = MockServer::start().await;
    let kube = Arc::new(LocalKube {
        object: Mutex::new(None),
        apply_updates: AtomicBool::new(true),
        suspended: AtomicBool::new(false),
        cleanup: AtomicBool::new(false),
        delete_requested: AtomicBool::new(false),
        generation: AtomicUsize::new(0),
        sandbox_uid: Uuid::new_v4().to_string(),
        pod_uid: Mutex::new(Uuid::new_v4().to_string()),
        pvc_uid: Uuid::new_v4().to_string(),
        pv_uid: Uuid::new_v4().to_string(),
        node_uid: Uuid::new_v4().to_string(),
    });
    let kube_state = kube.clone();
    Mock::given(method("PATCH"))
        .respond_with(move |request: &wiremock::Request| {
            if !kube_state.apply_updates.load(Ordering::SeqCst) {
                return ResponseTemplate::new(503);
            }
            let mut value: Value = serde_json::from_slice(&request.body).unwrap();
            let prior = kube_state.object.lock().unwrap().clone();
            if prior
                .as_ref()
                .is_some_and(|old| old["spec"]["operatingMode"] == "Suspended")
                && value["spec"]["operatingMode"] == "Running"
            {
                *kube_state.pod_uid.lock().unwrap() = Uuid::new_v4().to_string();
                kube_state.suspended.store(false, Ordering::SeqCst);
            }
            let generation = kube_state.generation.fetch_add(1, Ordering::SeqCst) + 1;
            value["metadata"]["uid"] = json!(kube_state.sandbox_uid);
            value["metadata"]["generation"] = json!(generation);
            value["metadata"]["resourceVersion"] = json!(generation.to_string());
            value["metadata"]["annotations"]["agents.x-k8s.io/pod-name"] = json!("node");
            *kube_state.object.lock().unwrap() = Some(value);
            ResponseTemplate::new(200).set_body_json(kube_state.current().unwrap())
        })
        .mount(&sandbox)
        .await;
    let kube_state = kube.clone();
    Mock::given(method("GET")).respond_with(move|request:&wiremock::Request| {
        let path=request.url.path();
        if path.ends_with("/nodes/local-node") {
            return ResponseTemplate::new(200).set_body_json(json!({"metadata":{"name":"local-node","uid":kube_state.node_uid,"resourceVersion":"9"},
                "status":{"nodeInfo":{"containerRuntimeVersion":"containerd://fixture-unverified"},"conditions":[{"type":"Ready","status":"True","lastHeartbeatTime":Utc::now().to_rfc3339()}]}}));
        }
        if path.ends_with("/persistentvolumes/local-pv") {
            if kube_state.current().is_none(){return ResponseTemplate::new(404);}
            return ResponseTemplate::new(200).set_body_json(json!({"metadata":{"name":"local-pv","uid":kube_state.pv_uid,"resourceVersion":"13"},
                "spec":{"claimRef":{"uid":kube_state.pvc_uid,"namespace":"nyxid-machine","name":"workspace"},"persistentVolumeReclaimPolicy":"Delete"},"status":{"phase":"Bound"}}));
        }
        for (plural,kind) in [("pods","Pod"),("persistentvolumeclaims","PersistentVolumeClaim")] {
            if path.ends_with(&format!("/{plural}")) {
                return ResponseTemplate::new(200).set_body_json(json!({"items":kube_state.resource(kind).into_iter().collect::<Vec<_>>() }));
            }
            if path.contains(&format!("/{plural}/")) {
                return kube_state.resource(kind).map(|v|ResponseTemplate::new(200).set_body_json(v)).unwrap_or_else(||ResponseTemplate::new(404));
            }
        }
        if path.ends_with("/sandboxes") {
            return ResponseTemplate::new(200).set_body_json(json!({"items":kube_state.current().into_iter().collect::<Vec<_>>() }));
        }
        kube_state.current().map(|v|ResponseTemplate::new(200).set_body_json(v)).unwrap_or_else(||ResponseTemplate::new(404))
    }).mount(&sandbox).await;
    let kube_state = kube.clone();
    Mock::given(method("DELETE"))
        .respond_with(move |_: &wiremock::Request| {
            kube_state.delete_requested.store(true, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(json!({"status":"Success"}))
        })
        .mount(&sandbox)
        .await;
    Mock::given(method("GET")).and(wiremock::matchers::path("/apis/storage.k8s.io/v1/storageclasses/local-fixture-csi"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"kind":"StorageClass","apiVersion":"storage.k8s.io/v1","metadata":{"name":"local-fixture-csi","uid":"storage-fixture"},"provisioner":"fixture.csi.example"})))
        .with_priority(1).mount(&sandbox).await;
    let manager = Manager {
        db: f.db.clone(),
        billing: Some(f.billing.clone()),
        api: nyxid_machine_updater::sandbox::SandboxApi::new(sandbox.uri().parse().unwrap(), None)
            .unwrap(),
        storage_class: Some("local-fixture-csi".into()),
        namespace: "nyxid-machine".into(),
        bootstrap_base_url: "https://nyxid.local.test".into(),
    };
    Journey {
        f,
        app,
        state,
        owner_token,
        address,
        server,
        _sandbox: sandbox,
        manager,
        kube,
        recipient,
        nonce,
    }
}
async fn create(j: &Journey, free: bool, key: &str) -> Value {
    let (status, quote) = http(
        j,
        "POST",
        "/api/v1/remote-machine-quotes",
        json!({"free_slot":free}),
        None,
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "quote: {quote}");
    assert_eq!(quote["profile"]["profile_version"], "remote-machine");
    assert_eq!(quote["profile"]["ephemeral_gib"], Value::Null);
    assert_eq!(quote["policy_version"], "remote-machine-mvp");
    assert_eq!(quote["policy"]["policy_version"], "remote-machine-mvp");
    assert_eq!(quote["profile"]["memory_mib"], 2048);

    if !free {
        assert_eq!(
            quote["credits_per_second"]
                .as_str()
                .unwrap()
                .parse::<Credits>()
                .unwrap(),
            "0.000040".parse::<Credits>().unwrap()
        );
    }
    let body = json!({"free_slot":free,"policy_version":"remote-machine-mvp","data_policy_version":"remote-machine-data","acknowledge_shared_environment":true,"quote_id":quote["quote_id"],"accept_account_balance":!free});
    let (created, replay_key) = if free {
        let (status, created) = http(
            j,
            "POST",
            "/api/v1/remote-machines",
            body.clone(),
            Some(key),
            true,
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "create: {created}");
        (created, key.to_owned())
    } else {
        let contender = format!("{key}-contender");
        let (a, b) = tokio::join!(
            http(
                j,
                "POST",
                "/api/v1/remote-machines",
                body.clone(),
                Some(key),
                true
            ),
            http(
                j,
                "POST",
                "/api/v1/remote-machines",
                body.clone(),
                Some(&contender),
                true
            )
        );
        assert_eq!(
            usize::from(a.0 == StatusCode::ACCEPTED) + usize::from(b.0 == StatusCode::ACCEPTED),
            1,
            "one atomic quote winner"
        );
        if a.0 == StatusCode::ACCEPTED {
            assert_eq!(b.0, StatusCode::CONFLICT);
            (a.1, key.to_owned())
        } else {
            assert_eq!(a.0, StatusCode::CONFLICT);
            (b.1, contender)
        }
    };
    assert_eq!(
        created["machine"]["profile"]["profile_version"],
        "remote-machine"
    );
    assert_eq!(created["machine"]["profile"]["ephemeral_gib"], Value::Null);
    assert_eq!(created["machine"]["policy_version"], "remote-machine-mvp");
    let held = wallet(&j.f).await.reserved_credits;
    let (status, replay) = http(
        j,
        "POST",
        "/api/v1/remote-machines",
        body.clone(),
        Some(&replay_key),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "replay: {replay}");
    assert_eq!(replay["machine"]["node_id"], created["machine"]["node_id"]);
    assert_eq!(replay["replayed"], true);
    assert_eq!(wallet(&j.f).await.reserved_credits, held);
    let mut changed = body;
    changed["free_slot"] = json!(!free);
    let (status, _) = http(
        j,
        "POST",
        "/api/v1/remote-machines",
        changed,
        Some(&replay_key),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    created
}
// This composed journey nests HTTP/router and Manager state machines. Heap-pin
// each bounded Manager pass so its large future is not stored inline again in
// the activate/journey futures on the test executor stack.
async fn activate(j: &Journey, id: &str) -> ManagedSocket {
    Box::pin(j.manager.tick()).await.unwrap();
    Box::pin(j.manager.tick()).await.unwrap();
    let bound = current(&j.f, id).await;
    assert!(
        bound.provider_coverage.is_none(),
        "ordinary native admission has no provider receipt"
    );
    assert!(bound.native_observation.as_ref().is_some_and(|o| o.running));
    assert!(effective_execution_cutoff(&bound).is_some());
    assert!(
        bound.writer_admitted,
        "current native facts admit only the bound writer"
    );
    assert_ne!(
        bound.resource_state,
        ResourceState::Ready,
        "Node WSS still establishes Ready"
    );
    let public = j.recipient.public_key_b64u().unwrap();
    let ticket = json!({"local_workload_nonce":j.nonce});
    let (status, prepared) = http(
        j,
        "POST",
        "/api/v1/machines/managed/prepare",
        json!({"node_id":id,"pod_uid":bound.pod_uid,"ticket":ticket,"recipient_public_key":public}),
        None,
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "public workload prepare");
    let managed: nyxid_machine::managed::Binding =
        serde_json::from_value(prepared["binding"].clone()).unwrap();
    let (status, delivery) = http(
        j,
        "POST",
        "/api/v1/machines/managed/bootstrap",
        json!({"binding":managed,"ticket":ticket,"recipient_public_key":public}),
        None,
        false,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "public sealed bootstrap");
    let envelope = serde_json::from_value(delivery["sealed"].clone()).unwrap();
    let plaintext = nyxid_machine::bootstrap::open(&envelope, &j.recipient, &managed).unwrap();
    let secrets: Value = serde_json::from_slice(&plaintext).unwrap();
    let (mut ws, _) = tokio_tungstenite::connect_async_with_config(
        format!("ws://{}/api/v1/nodes/ws", j.address),
        None,
        true,
    )
    .await
    .unwrap();
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"type":"auth","node_id":id,"token":secrets["auth_token"],"managed_runtime":managed})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    let message = tokio::time::timeout(std::time::Duration::from_secs(10), ws.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let reply: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
    assert_eq!(reply["type"], "auth_ok", "managed WSS authentication");
    let signing_key =
        Zeroizing::new(hex::decode(secrets["signing_secret"].as_str().unwrap()).unwrap());
    let window: nyxid_machine::managed::SignedRuntimeWindow =
        serde_json::from_value(reply["managed_runtime_window"].clone()).unwrap();
    window
        .verify(&managed, &signing_key, Utc::now().timestamp_millis())
        .expect("public auth response carries exact installed signed cutoff");
    let profile = nyxid_machine::MachineProfile {
        version: nyxid_machine::PROTOCOL_VERSION,
        runtime_id: Uuid::new_v4().to_string(),
        shell: true,
        files: true,
        browser: Some(false),
        roots: vec!["/workspace".into()],
        authority_versions: vec![2, 3],
        commands_isolated: Some(true),
        os: "linux".into(),
        arch: "aarch64".into(),
        ..Default::default()
    };
    ws.send(tokio_tungstenite::tungstenite::Message::Text(
        json!({"type":"status_update","capabilities":{"machine":profile}})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if current(&j.f, id).await.resource_state == ResourceState::Ready {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("current authenticated WSS promotes Ready");
    Box::pin(j.manager.tick()).await.unwrap();
    let (shutdown, mut stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(10));
        let mut last_revision = window.window.control_revision;
        loop {
            tokio::select! {
                _ = &mut stopped => {
                    let _ = ws.close(None).await;
                    break;
                }
                _ = heartbeat.tick() => {
                    if ws.send(tokio_tungstenite::tungstenite::Message::Text(
                        json!({"type":"heartbeat_pong"}).to_string().into()
                    )).await.is_err() {
                        break;
                    }
                }
                frame = ws.next() => {
                    let Some(Ok(frame)) = frame else { break; };
                    match frame {
                        tokio_tungstenite::tungstenite::Message::Text(text) => {
                            let value: Value = serde_json::from_str(&text).unwrap();
                            if value["type"] == "managed_runtime_window" {
                                let signed: nyxid_machine::managed::SignedRuntimeWindow =
                                    serde_json::from_value(value["window"].clone()).unwrap();
                                signed.verify(&managed, &signing_key, Utc::now().timestamp_millis())
                                    .expect("current signed managed runtime window");
                                assert!(signed.window.control_revision >= last_revision,
                                    "managed runtime cutoff control is monotonic");
                                last_revision = signed.window.control_revision;
                            }
                        }
                        tokio_tungstenite::tungstenite::Message::Close(_) => break,
                        _ => {}
                    }
                }
            }
        }
    });
    ManagedSocket { task, shutdown }
}
async fn action_http(j: &Journey, id: &str, action: &str) -> Value {
    let m = current(&j.f, id).await;
    let body = json!({"action":action,"expected_revision":m.revision,"data_policy_version":"remote-machine-data","acknowledge_data_loss":action=="delete"});
    let (status, result) = http(
        j,
        "POST",
        &format!("/api/v1/remote-machines/{id}/actions"),
        body,
        Some(&Uuid::new_v4().to_string()),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{action}: {result}");
    result
}
async fn cleanup(j: Journey) {
    let db = j.f.db.clone();
    drop(j);
    db.drop().await.unwrap();
}

async fn reach_renewal_margin(
    j: &Journey,
    id: &str,
    cutoff: chrono::DateTime<Utc>,
) -> AppResult<()> {
    tokio::time::timeout(
        std::time::Duration::from_secs((window_seconds() + 30) as u64),
        async {
            while Utc::now() + Duration::seconds(renewal_margin_seconds()) < cutoff {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                if Utc::now() + Duration::seconds(renewal_margin_seconds()) >= cutoff {
                    break;
                }
                Box::pin(j.manager.observe(&current(&j.f, id).await)).await?;
            }
            Ok(())
        },
    )
    .await
    .expect("UTC funding renewal margin reached within local fixture bound")
}

#[tokio::test]
async fn local_journey_paid_http_manager_billing() {
    let j = fixture_http("local_journey_paid").await;
    assert!(Arc::ptr_eq(
        &j.state.billing,
        j.manager.billing.as_ref().unwrap()
    ));
    let created = create(&j, false, "paid-public-create").await;
    let id = created["machine"]["node_id"].as_str().unwrap();
    assert_eq!(
        j.f.db
            .collection::<Document>(RUN_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        j.f.db
            .collection::<Document>(INTERVAL_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    let ws = Box::pin(activate(&j, id)).await;
    assert!(wallet(&j.f).await.reserved_credits > Credits::ZERO);
    // Record nonzero native observed running time before suspending the runtime.
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    Box::pin(j.manager.observe(&current(&j.f, id).await))
        .await
        .unwrap();
    let original = current(&j.f, id).await;
    let prior_installed = effective_execution_cutoff(&original).unwrap();
    assert_eq!(Some(prior_installed), original.funded_until);
    // observe() also renews at the margin. Block native application before
    // waiting, so either observer or funding pass can commit only the funds.
    j.kube.apply_updates.store(false, Ordering::SeqCst);
    if let Err(error) = reach_renewal_margin(&j, id, prior_installed).await {
        assert!(
            matches!(&error, AppError::Conflict(reason) if reason == "managed_platform_observation_pending"),
            "unexpected observation failure code {}",
            error.error_code()
        );
        assert!(Utc::now() + Duration::seconds(renewal_margin_seconds()) >= prior_installed);
        let pending = current(&j.f, id).await;
        assert_eq!(pending.resource_state, ResourceState::Ready);
        assert!(pending.funded_until.is_some_and(|at| at > prior_installed));
        assert_eq!(effective_execution_cutoff(&pending), Some(prior_installed));
    }
    // The preceding observations keep admission fresh while requesting money.
    Box::pin(j.manager.funding_tick()).await.unwrap();
    let funded = current(&j.f, id).await;
    assert_eq!(funded.resource_state, ResourceState::Ready);
    let newly_funded = funded.funded_until.unwrap();
    assert!(newly_funded > prior_installed);
    assert_eq!(
        effective_execution_cutoff(&funded),
        Some(prior_installed),
        "committed funds cannot substitute for applied native shutdownTime"
    );
    Box::pin(j.manager.tick()).await.unwrap();
    assert_eq!(
        effective_execution_cutoff(&current(&j.f, id).await),
        Some(prior_installed)
    );
    j.kube.apply_updates.store(true, Ordering::SeqCst);
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            Box::pin(j.manager.tick()).await.unwrap();
            if effective_execution_cutoff(&current(&j.f, id).await) == Some(newly_funded) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    })
    .await
    .expect("applied native shutdownTime extends only committed funding");
    j.f.lago.available.store(false, Ordering::SeqCst);
    // Establish the UTC margin precondition without rewriting persisted money
    // intervals or assuming one monotonic sleep proves a wall-clock deadline.
    let cutoff = current(&j.f, id).await.funded_until.unwrap();
    if let Err(error) = reach_renewal_margin(&j, id, cutoff).await {
        // observe() can cross the UTC margin during its awaits and attempt
        // renewal before the explicit funding pass. Lago is already unavailable.
        assert!(
            matches!(&error, AppError::Conflict(reason) if reason == "managed_platform_observation_pending"),
            "unexpected observation failure code {}",
            error.error_code()
        );
        assert!(Utc::now() + Duration::seconds(renewal_margin_seconds()) >= cutoff);
        let stopped = current(&j.f, id).await;
        assert_eq!(stopped.resource_state, ResourceState::Stopping);
        assert_eq!(
            stopped.stop_reason.as_deref(),
            Some("runtime_funding_unavailable")
        );
        assert_eq!(stopped.funded_until, Some(cutoff));
    }
    Box::pin(j.manager.funding_tick()).await.unwrap();
    let stopping = current(&j.f, id).await;
    assert_eq!(stopping.resource_state, ResourceState::Stopping);
    assert_eq!(
        stopping.stop_reason.as_deref(),
        Some("runtime_funding_unavailable")
    );
    assert_eq!(stopping.funded_until, Some(cutoff));
    assert!(
        effective_execution_cutoff(&stopping).is_none(),
        "stop removes execution coverage before native suspension completes"
    );
    assert!(require_accounting_final(&j.f.db, &stopping).await.is_err());
    assert!(wallet(&j.f).await.reserved_credits > Credits::ZERO);
    j.kube.suspended.store(true, Ordering::SeqCst);
    Box::pin(j.manager.tick()).await.unwrap();
    let paused = current(&j.f, id).await;
    assert_eq!(paused.resource_state, ResourceState::Paused);
    assert_eq!(wallet(&j.f).await.reserved_credits, Credits::ZERO);
    let run: MachineRuntimeRun =
        j.f.db
            .collection(RUN_COLLECTION)
            .find_one(doc! {"_id":&paused.runtime_run_id})
            .await
            .unwrap()
            .unwrap();
    assert!(run.cumulative_seconds > 0);
    assert_eq!(run.state, MachineRuntimeRunState::Closed);
    let ledger_before =
        j.f.db
            .collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap();
    assert!(ledger_before > 0);
    j.f.lago.available.store(true, Ordering::SeqCst);
    Box::pin(j.manager.funding_tick()).await.unwrap();
    assert_eq!(
        current(&j.f, id).await.resource_state,
        ResourceState::Paused,
        "provider balance recovery never resumes"
    );
    for suffix in ["/billing", "/billing/runs", "/usage"] {
        let (status, value) = http(
            &j,
            "GET",
            &format!("/api/v1/remote-machines/{id}{suffix}"),
            Value::Null,
            None,
            true,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{suffix}: {value}");
    }
    let old_node = paused.node_id.clone();
    let old_volume = paused.pvc_uid.clone();
    let old_generation = paused.runtime_generation;
    let (status, quote) = http(
        &j,
        "POST",
        "/api/v1/remote-machine-quotes",
        json!({"free_slot":false,"machine_id":id}),
        None,
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let resume_body = json!({"action":"resume","expected_revision":current(&j.f,id).await.revision,"policy_version":"remote-machine-mvp","data_policy_version":"remote-machine-data","acknowledge_shared_environment":true,"quote_id":quote["quote_id"],"accept_account_balance":true});
    let (status, resume) = http(
        &j,
        "POST",
        &format!("/api/v1/remote-machines/{id}/actions"),
        resume_body,
        Some("public-manual-resume"),
        true,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "new quote/manual resume: {resume}"
    );
    let resumed = current(&j.f, id).await;
    assert_eq!(resumed.node_id, old_node);
    assert_eq!(resumed.pvc_uid, old_volume);
    assert_ne!(resumed.runtime_run_id, paused.runtime_run_id);
    assert!(wallet(&j.f).await.reserved_credits > Credits::ZERO);
    assert_eq!(
        j.f.db
            .collection::<Document>(RUN_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
    Box::pin(j.manager.tick()).await.unwrap();
    assert!(current(&j.f, id).await.runtime_generation > old_generation);
    action_http(&j, id, "delete").await;
    j.kube.suspended.store(true, Ordering::SeqCst);
    Box::pin(j.manager.tick()).await.unwrap();
    assert_eq!(
        current(&j.f, id).await.resource_state,
        ResourceState::Deleting,
        "observable resource cleanup remains pending after accounting closes"
    );
    j.kube.cleanup.store(true, Ordering::SeqCst);
    Box::pin(j.manager.tick()).await.unwrap();
    assert_eq!(
        current(&j.f, id).await.resource_state,
        ResourceState::Deleted
    );
    Box::pin(j.manager.tick()).await.unwrap();
    assert_eq!(
        j.f.db
            .collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        ledger_before
    );
    ws.close().await;
    cleanup(j).await;
}

#[tokio::test]
async fn local_journey_free_http_cleanup_slot() {
    let j = fixture_http("local_journey_free").await;
    let created = create(&j, true, "free-public-create").await;
    let id = created["machine"]["node_id"].as_str().unwrap();
    let ws = Box::pin(activate(&j, id)).await;
    assert_eq!(
        j.f.db
            .collection::<Document>(RUN_COLLECTION)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(wallet(&j.f).await.reserved_credits, Credits::ZERO);
    action_http(&j, id, "pause").await;
    Box::pin(j.manager.tick()).await.unwrap();
    assert_eq!(
        current(&j.f, id).await.resource_state,
        ResourceState::Stopping
    );
    j.kube.suspended.store(true, Ordering::SeqCst);
    Box::pin(j.manager.tick()).await.unwrap();
    let paused = current(&j.f, id).await;
    assert_eq!(paused.resource_state, ResourceState::Paused);
    let (_, entitlement) = http(
        &j,
        "GET",
        "/api/v1/remote-machine-entitlement",
        Value::Null,
        None,
        true,
    )
    .await;
    assert_eq!(entitlement["free_slot_available"], false);
    assert_eq!(entitlement["machine_id"], id);
    action_http(&j, id, "delete").await;
    Box::pin(j.manager.tick()).await.unwrap();
    assert_eq!(
        current(&j.f, id).await.resource_state,
        ResourceState::Deleting
    );
    let (_, entitlement) = http(
        &j,
        "GET",
        "/api/v1/remote-machine-entitlement",
        Value::Null,
        None,
        true,
    )
    .await;
    assert_eq!(entitlement["free_slot_available"], false);
    j.kube.cleanup.store(true, Ordering::SeqCst);
    Box::pin(j.manager.tick()).await.unwrap();
    assert_eq!(
        current(&j.f, id).await.resource_state,
        ResourceState::Deleted
    );
    let (_, entitlement) = http(
        &j,
        "GET",
        "/api/v1/remote-machine-entitlement",
        Value::Null,
        None,
        true,
    )
    .await;
    assert_eq!(entitlement["free_slot_available"], true);
    Box::pin(j.manager.tick()).await.unwrap();
    assert_eq!(
        current(&j.f, id).await.resource_state,
        ResourceState::Deleted
    );
    let next = create(&j, true, "next-free-public-create").await;
    assert_ne!(next["machine"]["node_id"], id);
    ws.close().await;
    cleanup(j).await;
}

#[tokio::test]
async fn current_saved_image_and_runtime_resume_rekey_survive_new_deployment() {
    use crate::models::remote_machine::POLICY_VERSION;
    let j = fixture_http("current_saved_contract").await;
    let node_id = Uuid::new_v4().to_string();
    let mut old = CreateMachineInput::fixed(
        &j.f.owner,
        false,
        &format!("old-registry/node@sha256:{}", "b".repeat(64)),
    )
    .preview(&node_id, Utc::now());
    old.profile.runtime_class = Some("saved-runtime-a".into());
    old.resource_state = ResourceState::Paused;
    old.runtime_generation = 1;
    old.sandbox_uid = Some(Uuid::new_v4().to_string());
    old.pvc_name = Some("original-workspace".into());
    old.pvc_uid = Some(Uuid::new_v4().to_string());
    j.f.db
        .collection::<ManagedMachine>(MACHINES)
        .insert_one(&old)
        .await
        .unwrap();
    let (status, quoted) = http(
        &j,
        "POST",
        "/api/v1/remote-machine-quotes",
        json!({"free_slot":false,"machine_id":node_id}),
        None,
        true,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{quoted}");
    assert_eq!(
        quoted["profile"]["profile_version"],
        nyxid_machine::managed::PROFILE
    );
    assert_eq!(quoted["profile"]["ephemeral_gib"], serde_json::Value::Null);
    assert_eq!(
        old.profile.runtime_class.as_deref(),
        Some("saved-runtime-a")
    );
    assert_eq!(quoted["policy_version"], POLICY_VERSION);
    assert_eq!(quoted["policy"]["policy_version"], POLICY_VERSION);
    assert_eq!(
        quoted["quota"]["profile_version"],
        nyxid_machine::managed::PROFILE
    );
    assert_eq!(quoted["quota"]["operation"], "retained_volume_resume");
    let quote: MachineRuntimeQuote =
        j.f.db
            .collection(QUOTE_COLLECTION)
            .find_one(doc! {"_id":quoted["quote_id"].as_str().unwrap()})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        quote.terms,
        terms(&old.profile, &old.policy_version).unwrap()
    );
    let new_create = json!({"free_slot":false,"policy_version":POLICY_VERSION,"data_policy_version":"remote-machine-data","acknowledge_shared_environment":true,"quote_id":quoted["quote_id"],"accept_account_balance":true});
    let (status, rejected) = http(
        &j,
        "POST",
        "/api/v1/remote-machines",
        new_create,
        Some("old-quote-new-allocation"),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{rejected}");
    assert_eq!(wallet(&j.f).await.reserved_credits, Credits::ZERO);
    let mut resume = json!({"action":"resume","expected_revision":old.revision,"policy_version":"remote-machine-mvp-v1","data_policy_version":"remote-machine-data","acknowledge_shared_environment":true,"quote_id":quoted["quote_id"],"accept_account_balance":true});
    let endpoint = format!("/api/v1/remote-machines/{node_id}/actions");
    let (status, _) = http(
        &j,
        "POST",
        &endpoint,
        resume.clone(),
        Some("wrong-current-policy"),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    resume["policy_version"] = json!(POLICY_VERSION);
    let (status, accepted) = http(
        &j,
        "POST",
        &endpoint,
        resume.clone(),
        Some("saved-policy-resume"),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let resumed = current(&j.f, &node_id).await;
    assert_eq!(resumed.profile, old.profile);
    assert_eq!(resumed.policy_version, old.policy_version);
    assert_eq!(resumed.pvc_name, old.pvc_name);
    assert_eq!(resumed.pvc_uid, old.pvc_uid);
    let run: MachineRuntimeRun =
        j.f.db
            .collection(RUN_COLLECTION)
            .find_one(doc! {"_id":&resumed.runtime_run_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(run.terms, terms(&old.profile, &old.policy_version).unwrap());
    let held = wallet(&j.f).await.reserved_credits;
    let (status, replay) = http(
        &j,
        "POST",
        &endpoint,
        resume.clone(),
        Some("saved-policy-resume"),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{replay}");
    assert_eq!(wallet(&j.f).await.reserved_credits, held);
    resume["quote_id"] = json!(Uuid::new_v4().to_string());
    let (status, _) = http(
        &j,
        "POST",
        &endpoint,
        resume,
        Some("saved-policy-resume"),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    // Rekey uses the saved current image/RuntimeClass/PVC contract for free Machines too.
    let free_id = Uuid::new_v4().to_string();
    let mut free_old = old.clone();
    free_old.node_id = free_id.clone();
    free_old.sandbox_name =
        crate::services::remote_machine_adapter::SandboxIntent::deterministic_name(&free_id);
    free_old.free_slot = true;
    free_old.billing_state = BillingState::Free;
    free_old.resource_state = ResourceState::Ready;
    j.f.db
        .collection::<ManagedMachine>(MACHINES)
        .insert_one(&free_old)
        .await
        .unwrap();
    let endpoint = format!("/api/v1/remote-machines/{free_id}/rekey");
    let mut rekey = json!({"expected_revision":free_old.revision,"policy_version":"remote-machine-mvp-v2","data_policy_version":"remote-machine-data","acknowledge_shared_environment":true});
    let (status, _) = http(
        &j,
        "POST",
        &endpoint,
        rekey.clone(),
        Some("free-old-wrong-policy"),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    rekey["policy_version"] = json!(POLICY_VERSION);
    let (status, accepted) = http(
        &j,
        "POST",
        &endpoint,
        rekey.clone(),
        Some("free-old-rekey"),
        true,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let rekeyed = current(&j.f, &free_id).await;
    assert_eq!(rekeyed.profile, free_old.profile);
    assert_eq!(rekeyed.pvc_uid, free_old.pvc_uid);
    let (status, _) = http(&j, "POST", &endpoint, rekey, Some("free-old-rekey"), true).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    cleanup(j).await;
}

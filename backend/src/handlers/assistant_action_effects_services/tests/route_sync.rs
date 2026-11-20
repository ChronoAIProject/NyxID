use super::*;
use crate::handlers::user_services_handler;
use crate::models::{
    downstream_service::{COLLECTION_NAME as CATALOG, test_helpers::dummy_service},
    node_service_binding::{COLLECTION_NAME as BINDINGS, NodeServiceBinding},
};
use futures::TryStreamExt;
use mongodb::event::{EventHandler, command::CommandEvent};
use std::sync::{Arc, Mutex};

struct Routes {
    db: mongodb::Database,
    actor: String,
    service: UserService,
    catalog: String,
    old: String,
    new: String,
}

impl Routes {
    async fn seed(sibling: bool) -> Self {
        let (db, actor) = prepare_transaction_actor("route_sync").await;
        db.collection::<NodeServiceBinding>(BINDINGS)
            .create_index(
                IndexModel::builder()
                    .keys(doc! {"node_id":1,"service_id":1})
                    .options(
                        IndexOptions::builder()
                            .unique(true)
                            .partial_filter_expression(doc! {"is_active":true})
                            .build(),
                    )
                    .build(),
            )
            .await
            .unwrap();
        let mut catalog = dummy_service();
        catalog.id = Uuid::new_v4().to_string();
        catalog.slug = "route-sync".into();
        catalog.auth_method = "none".into();
        catalog.base_url = "https://127.0.0.1".into();
        db.collection::<crate::models::downstream_service::DownstreamService>(CATALOG)
            .insert_one(&catalog)
            .await
            .unwrap();
        let old = Uuid::new_v4().to_string();
        let new = Uuid::new_v4().to_string();
        for id in [&old, &new] {
            db.collection(NODES)
                .insert_one(test_node(id, &actor))
                .await
                .unwrap();
        }
        let service = seed_service(&db, &actor, false, Some(&old)).await.0;
        if sibling {
            seed_service_named(&db, &actor, "sibling", false, Some(&old)).await;
        }
        db.collection::<bson::Document>(USER_SERVICES)
            .update_many(
                doc! {},
                doc! {"$set":{"catalog_service_id": &catalog.id, "node_priority": 23}},
            )
            .await
            .unwrap();
        let now = chrono::Utc::now();
        db.collection(BINDINGS)
            .insert_one(NodeServiceBinding {
                id: Uuid::new_v4().to_string(),
                user_id: actor.clone(),
                service_id: catalog.id.clone(),
                node_id: old.clone(),
                is_active: true,
                priority: 7,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        Self {
            db,
            actor,
            service,
            catalog: catalog.id,
            old,
            new,
        }
    }

    fn router(&self, db: mongodb::Database) -> (Router, String) {
        let state = test_app_state(db);
        let token = access_token(&state, &self.actor);
        let edits = Router::new()
            .route(
                "/user-services/{id}",
                axum::routing::put(user_services_handler::update_user_service)
                    .delete(user_services_handler::delete_user_service),
            )
            .route(
                "/keys/{id}",
                axum::routing::put(keys::update_key).delete(keys::delete_key),
            )
            .route("/keys", axum::routing::post(keys::create_key))
            .layer(axum::middleware::from_fn(
                crate::services::service_history::context::middleware,
            ))
            .with_state(state.clone());
        (app(state).merge(edits), token)
    }

    async fn edit(
        &self,
        app: Router,
        token: &str,
        assistant: bool,
        node: Option<&str>,
    ) -> (StatusCode, Value) {
        if assistant {
            request(
                app,
                token,
                "POST",
                "/assistant/actions/services/route",
                Some(json!({
                    "actionRequestId": if node.is_some() { "route-move" } else { "route-clear" },
                    "userServiceId": self.service.id,
                    "viaNodeId": node,
                })),
            )
            .await
        } else {
            request(
                app,
                token,
                "PUT",
                &format!("/user-services/{}", self.service.id),
                Some(json!({
                    "node_id": node.unwrap_or("")
                })),
            )
            .await
        }
    }

    async fn bindings(&self) -> Vec<(String, i32)> {
        let rows: Vec<NodeServiceBinding> = self
            .db
            .collection(BINDINGS)
            .find(doc! {"user_id":&self.actor, "service_id":&self.catalog,"is_active":true})
            .sort(doc! {"priority":1})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        rows.into_iter()
            .map(|row| (row.node_id, row.priority))
            .collect()
    }
}

#[tokio::test]
async fn assistant_and_user_services_move_and_clear_the_same_bindings() {
    for sibling in [false, true] {
        for assistant in [false, true] {
            let f = Routes::seed(sibling).await;
            let (router, token) = f.router(f.db.clone());
            let retained: Vec<_> = sibling.then(|| (f.old.clone(), 7)).into_iter().collect();
            for target in [Some(f.new.as_str()), None] {
                let (status, body) = f.edit(router.clone(), &token, assistant, target).await;
                assert_eq!(status, StatusCode::OK, "{body}");
                let mut expected: Vec<_> =
                    target.map(|id| (id.to_owned(), 0)).into_iter().collect();
                expected.extend(retained.clone());
                assert_eq!(
                    f.bindings().await,
                    expected,
                    "assistant={assistant} sibling={sibling} target={target:?}"
                );
                if assistant {
                    let (status, replay) = f.edit(router.clone(), &token, true, target).await;
                    assert_eq!(status, StatusCode::OK, "{replay}");
                    assert_eq!(body["replayed"], false);
                    assert_eq!(replay["replayed"], true);
                    let completed = if target.is_some() { 1 } else { 2 };
                    assert_eq!(f.db.collection::<bson::Document>(ASSISTANT_ACTION_RECEIPTS)
                        .count_documents(doc!{"user_id":&f.actor, "action":SERVICE_ROUTE_ACTION, "status":"completed"})
                        .await.unwrap(), completed);
                    assert_eq!(f.db.collection::<bson::Document>("service_change_events")
                        .count_documents(doc!{"service_id":&f.service.id, "action":"service.routing_changed","actor.id":&f.actor})
                        .await.unwrap(), completed);
                    let service: UserService =
                        f.db.collection(USER_SERVICES)
                            .find_one(doc! {"_id":&f.service.id})
                            .await
                            .unwrap()
                            .unwrap();
                    assert_eq!(service.node_id.as_deref(), target);
                    assert_eq!(service.node_priority, 23);
                    assert_eq!(service.state_version as u64, 1 + completed);
                }
            }
        }
    }
}

async fn observed(
    db: &mongodb::Database,
    app_name: &str,
    handler: EventHandler<CommandEvent>,
) -> mongodb::Database {
    let mut options =
        mongodb::options::ClientOptions::parse(crate::test_utils::selected_test_database_uri())
            .await
            .unwrap();
    options.app_name = Some(app_name.into());
    options.command_event_handler = Some(handler);
    mongodb::Client::with_options(options)
        .unwrap()
        .database(db.name())
}

fn pause_before_write(
    collection: &'static str,
    commands: &'static [&'static str],
) -> (
    EventHandler<CommandEvent>,
    tokio::sync::oneshot::Receiver<()>,
    std::sync::mpsc::Sender<()>,
) {
    let (arrived, wait) = tokio::sync::oneshot::channel();
    let (resume, paused) = std::sync::mpsc::channel();
    let pause = Mutex::new(Some((arrived, paused)));
    let monitor = EventHandler::callback(move |event| {
        if let CommandEvent::Started(e) = event
            && commands.contains(&e.command_name.as_str())
            && e.command.get_str(&e.command_name) == Ok(collection)
            && let Some((arrived, paused)) = pause.lock().unwrap().take()
        {
            arrived.send(()).unwrap();
            let _ = paused.recv();
        }
    });
    (monitor, wait, resume)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_route_edit_uses_the_committed_predecessor() {
    overlap(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_last_siblings_both_leaving_retire_the_binding() {
    overlap(true).await;
}

async fn overlap(sibling: bool) {
    let f = Routes::seed(sibling).await;
    for name in ["service_change_events", "service_history_heads"] {
        if !f
            .db
            .list_collection_names()
            .await
            .unwrap()
            .iter()
            .any(|n| n == name)
        {
            f.db.create_collection(name).await.unwrap();
        }
    }
    let (monitor, wait, resume) = pause_before_write(USER_SERVICES, &["update", "findAndModify"]);
    let db = observed(&f.db, "assistant-route-race", monitor).await;
    let (router, token) = f.router(db);
    let service_id = f.service.id.clone();
    let mut task = tokio::spawn(async move {
        request(
            router,
            &token,
            "POST",
            "/assistant/actions/services/route",
            Some(json!({
                "userServiceId":service_id, "actionRequestId":"route-race"
            })),
        )
        .await
    });
    tokio::select! {
        result = &mut task => panic!("assistant returned before its write: {result:?}"),
        result = wait => result.unwrap(),
    }
    let (router, token) = f.router(f.db.clone());
    let (status, body) = if sibling {
        let other: UserService =
            f.db.collection(USER_SERVICES)
                .find_one(doc! {"slug":"sibling"})
                .await
                .unwrap()
                .unwrap();
        request(
            router,
            &token,
            "PUT",
            &format!("/user-services/{}", other.id),
            Some(json!({"node_id":""})),
        )
        .await
    } else {
        f.edit(router, &token, false, Some(&f.new)).await
    };
    resume.send(()).unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = task.await.unwrap();
    assert_eq!(status, StatusCode::OK, "{body}");
    let stored: UserService =
        f.db.collection(USER_SERVICES)
            .find_one(doc! {"_id": &f.service.id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(stored.node_id, None);
    assert!(f.bindings().await.is_empty());
}

#[tokio::test]
async fn routed_request_and_heartbeat_command_counts() {
    use crate::services::{
        billing::route_inventory::internal_node_dispatch_permit,
        node_routing_service,
        node_ws_manager::{NodeProxyRequest, NodeProxyResponse, ProxyResponseType},
    };
    let f = Routes::seed(false).await;
    let trace = Arc::new(Mutex::new(Vec::new()));
    let record = trace.clone();
    let db = observed(
        &f.db,
        "route-sync-commands",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(e) = event {
                record.lock().unwrap().push(e.command_name);
            }
        }),
    )
    .await;
    let state = test_app_state(db);
    let (tx, mut rx) = tokio::sync::mpsc::channel(2);
    crate::test_utils::register_test_node_connection(&state, &f.old, tx).await;
    trace.lock().unwrap().clear();
    let route = node_routing_service::resolve_node_route(
        &state.db,
        &f.actor,
        &f.catalog,
        &state.node_ws_manager,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(route.node_id, f.old);
    let resolution = std::mem::take(&mut *trace.lock().unwrap());
    let id = Uuid::new_v4().to_string();
    let req = NodeProxyRequest {
        target_id: None,
        request_id: id.clone(),
        service_id: f.catalog.clone(),
        service_slug: f.service.slug,
        base_url: "https://example.invalid".into(),
        method: "GET".into(),
        path: "/".into(),
        query: None,
        headers: vec![].into(),
        body: None,
    };
    let sent = state.node_dispatch.send_proxy_request_classified(
        &f.old,
        req,
        None,
        internal_node_dispatch_permit(),
    );
    tokio::pin!(sent);
    let outbound = tokio::select! {
        _ = &mut sent => panic!("dispatch finished before the node received its request"),
        message = rx.recv() => message.unwrap(),
    };
    let crate::services::node_ws_manager::NodeOutboundMessage::Text(text) = outbound else {
        panic!("expected a proxy request");
    };
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap()["request_id"],
        id
    );
    state.node_ws_manager.deliver_proxy_response(
        &f.old,
        NodeProxyResponse {
            request_id: id,
            status: 200,
            headers: vec![],
            body: vec![],
        },
    );
    assert!(matches!(sent.await, Ok(ProxyResponseType::Complete(r)) if r.status == 200));
    let dispatch = std::mem::take(&mut *trace.lock().unwrap());
    crate::handlers::node_ws::node_ws_manager_heartbeat_sweep(
        &state.db,
        &state.node_ws_manager,
        &state.replica_identity,
        i64::MAX as u64,
    )
    .await;
    assert!(rx.try_recv().is_ok());
    let heartbeat = std::mem::take(&mut *trace.lock().unwrap());
    println!("resolution={resolution:?}, dispatch={dispatch:?}, heartbeat={heartbeat:?}");
    assert_eq!(resolution, ["find"; 5]);
    assert_eq!(dispatch, ["find"]);
    assert_eq!(heartbeat, ["find", "update"]);
}

#[derive(Clone, Copy, Debug)]
enum Operation {
    UserPut,
    KeyPut,
    Assistant,
    UserDelete,
    Disconnect,
    Cascade,
    Pending,
    Create,
}

impl Operation {
    async fn setup(self) -> Routes {
        let f = Routes::seed(matches!(self, Self::Cascade)).await;
        if matches!(self, Self::Pending | Self::Cascade) {
            let mut provider = grant_provider_for_test();
            provider.revocation.as_mut().unwrap().url = "https://example.invalid/revoke".into();
            f.db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
                .insert_one(&provider)
                .await
                .unwrap();
            let services: Vec<UserService> =
                f.db.collection(USER_SERVICES)
                    .find(doc! {})
                    .await
                    .unwrap()
                    .try_collect()
                    .await
                    .unwrap();
            for service in services {
                let mut key = fixture_api_key(&Uuid::new_v4().to_string(), &f.actor);
                key.provider_config_id = Some(provider.id.clone());
                key.credential_type = "oauth2".into();
                key.credential_encrypted = None;
                if matches!(self, Self::Pending) {
                    key.status = "pending_auth".into();
                } else {
                    key.credential_source = Some("platform".into());
                    key.connection_id = Some(Uuid::new_v4().to_string());
                }
                f.db.collection::<UserService>(USER_SERVICES)
                    .update_one(
                        doc! {"_id":&service.id},
                        doc! {"$set":{"api_key_id":&key.id,"auth_method":"oauth2"}},
                    )
                    .await
                    .unwrap();
                f.db.collection(USER_API_KEYS)
                    .insert_one(key)
                    .await
                    .unwrap();
            }
            if matches!(self, Self::Pending) {
                let now = bson::DateTime::now();
                f.db.collection("user_provider_tokens")
                    .insert_one(doc! {
                        "_id": Uuid::new_v4().to_string(), "user_id": &f.actor,
                        "provider_config_id": &provider.id, "token_type": "oauth2",
                        "status": "active", "created_at": now, "updated_at": now,
                    })
                    .await
                    .unwrap();
            }
        }
        f
    }

    async fn apply(self, f: &Routes, db: mongodb::Database) -> (StatusCode, Value) {
        let (router, token) = f.router(db);
        let key = format!("/keys/{}", f.service.id);
        match self {
            Self::UserPut | Self::Assistant => {
                f.edit(
                    router,
                    &token,
                    matches!(self, Self::Assistant),
                    Some(&f.new),
                )
                .await
            }
            Self::KeyPut => {
                request(router, &token, "PUT", &key, Some(json!({"node_id": f.new}))).await
            }
            Self::UserDelete => {
                let url = format!("/user-services/{}", f.service.id);
                request(router, &token, "DELETE", &url, None).await
            }
            Self::Disconnect => request(router, &token, "DELETE", &key, None).await,
            Self::Cascade => {
                request(
                    router,
                    &token,
                    "DELETE",
                    &format!("{key}?cascade_grant=true"),
                    None,
                )
                .await
            }
            Self::Pending => {
                request(
                    router,
                    &token,
                    "DELETE",
                    &format!("{key}?only_if_pending=true"),
                    None,
                )
                .await
            }
            Self::Create => {
                let body = json!({"label":"Second", "slug":"second",
                    "service_slug":"route-sync", "node_id":f.new});
                request(router, &token, "POST", "/keys", Some(body)).await
            }
        }
    }

    async fn assert_state(self, f: &Routes) -> bool {
        let active =
            f.db.collection::<UserService>(USER_SERVICES)
                .find(doc! {"user_id":&f.actor,"catalog_service_id":&f.catalog,"is_active":true})
                .await
                .unwrap()
                .try_collect::<Vec<_>>()
                .await
                .unwrap();
        let mut primaries: Vec<_> = active.iter().map(|s| s.node_id.clone().unwrap()).collect();
        primaries.sort();
        let initial_count = if matches!(self, Self::Cascade) { 2 } else { 1 };
        let old = primaries == vec![f.old.clone(); initial_count];
        let mut complete = match self {
            Self::UserPut | Self::KeyPut | Self::Assistant => vec![f.new.clone()],
            Self::Create => vec![f.new.clone(), f.old.clone()],
            _ => vec![],
        };
        complete.sort();
        assert!(
            old || primaries == complete,
            "{self:?} unexpected primaries {primaries:?}"
        );
        let mut bindings: Vec<_> = f.bindings().await.into_iter().map(|(id, _)| id).collect();
        bindings.sort();
        primaries.dedup();
        assert_eq!(primaries, bindings, "{self:?} exposed a partial route");
        if matches!(self, Self::Pending) {
            let key: UserApiKey =
                f.db.collection(USER_API_KEYS)
                    .find_one(doc! {})
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(key.status, if old { "pending_auth" } else { "revoked" });
            let token =
                f.db.collection::<bson::Document>("user_provider_tokens")
                    .find_one(doc! {})
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(
                token.get_str("status").unwrap(),
                if old { "active" } else { "revoked" }
            );
        }
        !old
    }
}

const WRITES: &[&str] = &[
    "insert",
    "update",
    "findAndModify",
    "delete",
    "commitTransaction",
];

#[derive(Default)]
struct Writes {
    count: usize,
    failed: bool,
    commit_session: Option<bson::Document>,
}

async fn monitored_writes(f: &Routes) -> (mongodb::Database, String, Arc<Mutex<Writes>>) {
    let writes = Arc::new(Mutex::new(Writes::default()));
    let recorded = writes.clone();
    let name = Uuid::new_v4().to_string();
    let db = observed(
        &f.db,
        &name,
        EventHandler::callback(move |event| {
            let mut w = recorded.lock().unwrap();
            match event {
                CommandEvent::Started(e) if WRITES.contains(&e.command_name.as_str()) => {
                    w.count += 1;
                    if e.command_name == "commitTransaction" {
                        w.commit_session = e.command.get_document("lsid").ok().cloned();
                    }
                }
                CommandEvent::Failed(e) if WRITES.contains(&e.command_name.as_str()) => {
                    w.failed = true
                }
                _ => {}
            }
        }),
    )
    .await;
    (db, name, writes)
}

#[tokio::test]
async fn route_write_boundaries() {
    for op in [
        Operation::UserPut,
        Operation::KeyPut,
        Operation::Assistant,
        Operation::UserDelete,
        Operation::Disconnect,
        Operation::Cascade,
        Operation::Pending,
        Operation::Create,
    ] {
        let f = op.setup().await;
        let (db, _, writes) = monitored_writes(&f).await;
        let (status, body) = op.apply(&f, db).await;
        assert!(status.is_success(), "{op:?} {status} {body}");
        assert!(op.assert_state(&f).await);
        crate::services::shutdown::process().idle().await;
        let n = writes.lock().unwrap().count;
        assert!(n >= 2);
        for cutoff in 0..n {
            let f = op.setup().await;
            let (db, name, writes) = monitored_writes(&f).await;
            let admin = f.db.client().database("admin");
            let mode = if cutoff == 0 {
                bson::Bson::String("alwaysOn".into())
            } else {
                doc! {"skip":cutoff as i32}.into()
            };
            admin
                .run_command(doc! {
                    "configureFailPoint":"failCommand","mode":mode,
                    "data":{"failCommands":WRITES,"appName":name,"errorCode":2}
                })
                .await
                .unwrap();
            let _ = op.apply(&f, db).await;
            crate::services::shutdown::process().idle().await;
            admin
                .run_command(doc! {"configureFailPoint":"failCommand","mode":"off"})
                .await
                .unwrap();
            let session = writes.lock().unwrap().commit_session.clone();
            if let Some(lsid) = session {
                // A rejected commit can leave an uncommitted server transaction.
                admin
                    .run_command(doc! {"killSessions":[lsid]})
                    .await
                    .unwrap();
            }
            assert!(
                writes.lock().unwrap().failed,
                "{op:?}: missed boundary {cutoff}/{n}"
            );
            op.assert_state(&f).await;
        }
        println!("{op:?}: {n} write boundaries, plus successful completion");
    }
}

mod cross_owner;

#[tokio::test]
async fn clearing_a_shared_primary_preserves_independent_fallback_priorities() {
    for assistant in [false, true] {
        let f = Routes::seed(true).await;
        let mut fallback: NodeServiceBinding =
            f.db.collection(BINDINGS)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
        fallback.id = Uuid::new_v4().to_string();
        fallback.node_id = f.new.clone();
        fallback.priority = -3;
        f.db.collection(BINDINGS)
            .insert_one(fallback)
            .await
            .unwrap();
        let (router, token) = f.router(f.db.clone());
        for target in [None, Some(f.new.as_str())] {
            let (status, body) = f.edit(router.clone(), &token, assistant, target).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(
                f.bindings().await,
                [(f.new.clone(), -3), (f.old.clone(), 7)]
            );
        }
    }
}

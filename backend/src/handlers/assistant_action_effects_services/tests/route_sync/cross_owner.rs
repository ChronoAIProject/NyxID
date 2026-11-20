use super::*;
use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgRole};
use crate::test_utils::{
    register_test_node_connection, set_test_approval_preference, test_membership,
};

async fn organization(f: &Routes) -> String {
    let org = Uuid::new_v4().to_string();
    f.db.collection(USERS)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    f.db.collection(MEMBERSHIPS)
        .insert_one(test_membership(&org, &f.actor, OrgRole::Admin, None))
        .await
        .unwrap();
    org
}

async fn create_connection(
    router: Router,
    token: &str,
    owner: Option<&str>,
    slug: &str,
    node: Option<&str>,
) -> String {
    let (status, body) = request(router, token, "POST", "/keys", Some(json!({
        "label":slug, "slug":slug, "service_slug":"route-sync", "target_org_id":owner, "node_id":node,
    }))).await;
    assert!(status.is_success(), "create {slug}: {status} {body}");
    body["id"].as_str().unwrap().to_owned()
}

async fn sole_binding(f: &Routes, node: &str) -> bson::RawDocumentBuf {
    let rows: Vec<bson::RawDocumentBuf> =
        f.db.collection(BINDINGS)
            .find(doc! {"node_id":node, "service_id":&f.catalog})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    let [row]: [_; 1] = rows
        .try_into()
        .expect("one binding for the pair, including inactive rows");
    row
}

async fn assert_org_route(
    f: &Routes,
    org: &str,
    id: &str,
    node: Option<&str>,
    priority: i32,
    original: &bson::RawDocumentBuf,
) {
    let service: UserService =
        f.db.collection(USER_SERVICES)
            .find_one(doc! {"_id":id, "user_id":org, "is_active":true})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(service.node_id.as_deref(), node);
    assert_eq!(service.node_priority, priority);
    let actual = sole_binding(f, &f.old).await;
    assert_eq!(actual.as_bytes(), original.as_bytes());
    let org_nodes =
        f.db.collection::<bson::Document>(BINDINGS)
            .distinct(
                "node_id",
                doc! {"user_id":org,"service_id":&f.catalog,"is_active":true},
            )
            .await
            .unwrap();
    let expected: Vec<bson::Bson> = node
        .filter(|n| *n != f.old)
        .map(Into::into)
        .into_iter()
        .collect();
    assert_eq!(org_nodes, expected);
}

async fn edit(router: &Router, token: &str, path: &str, body: Value) {
    let method = if path.starts_with("/assistant/") {
        "POST"
    } else {
        "PUT"
    };
    let (status, response) = request(router.clone(), token, method, path, Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{method} {path}: {response}");
}

#[tokio::test]
async fn org_admin_put_user_services_preserves_personal_binding() {
    org_admin_route(Operation::UserPut).await;
}

#[tokio::test]
async fn org_admin_put_keys_preserves_personal_binding() {
    org_admin_route(Operation::KeyPut).await;
}

#[tokio::test]
async fn org_admin_assistant_route_preserves_personal_binding() {
    org_admin_route(Operation::Assistant).await;
}

#[tokio::test]
async fn org_admin_create_key_preserves_personal_binding() {
    org_admin_route(Operation::Create).await;
}

async fn org_admin_route(op: Operation) {
    let f = Routes::seed(false).await;
    let org = organization(&f).await;
    let original = sole_binding(&f, &f.old).await;
    assert_eq!(original.get_str("user_id").unwrap(), f.actor);
    assert!(original.get_bool("is_active").unwrap());
    let (router, token) = f.router(f.db.clone());
    let create_node = matches!(op, Operation::Create).then_some(f.old.as_str());
    let id = create_connection(router.clone(), &token, Some(&org), "org-route", create_node).await;
    let service_path = format!("/user-services/{id}");
    let key_path = format!("/keys/{id}");
    let assistant_path = "/assistant/actions/services/route";
    // Keys PUT and the assistant accept a node only. Seed a nondefault
    // connection priority so resetting it would be visible.
    if matches!(op, Operation::KeyPut | Operation::Assistant) {
        f.db.collection::<UserService>(USER_SERVICES)
            .update_one(doc! {"_id":&id}, doc! {"$set":{"node_priority":11}})
            .await
            .unwrap();
    }
    let priority = match op {
        Operation::UserPut => {
            edit(
                &router,
                &token,
                &service_path,
                json!({"node_id":f.old,"node_priority":23}),
            )
            .await;
            23
        }
        Operation::KeyPut => {
            edit(&router, &token, &key_path, json!({"node_id":f.old})).await;
            11
        }
        Operation::Assistant => {
            edit(
                &router,
                &token,
                assistant_path,
                json!({"actionRequestId":"org-pin", "userServiceId":id, "viaNodeId":f.old}),
            )
            .await;
            11
        }
        Operation::Create => 0,
        _ => unreachable!(),
    };
    assert_org_route(&f, &org, &id, Some(&f.old), priority, &original).await;
    let action = if matches!(op, Operation::Create) {
        "service.created"
    } else {
        "service.routing_changed"
    };
    let events =
        f.db.collection::<bson::Document>("service_change_events")
            .count_documents(doc! {"service_id":&id, "action":action, "actor.id":&f.actor})
            .await
            .unwrap();
    assert_eq!(events, 1);
    if matches!(op, Operation::Create) {
        dispatch_org_slug(&f, &org).await;
    }

    edit(&router, &token, &service_path, json!({"node_id":f.new})).await;
    assert_org_route(&f, &org, &id, Some(&f.new), priority, &original).await;
    edit(&router, &token, &key_path, json!({"node_id":f.old})).await;
    assert_org_route(&f, &org, &id, Some(&f.old), priority, &original).await;
    let clear = json!({"actionRequestId":"org-clear", "userServiceId":id});
    if matches!(op, Operation::Create) {
        // This connection was created with its endpoint stored only on the node.
        let (status, error) =
            request(router.clone(), &token, "POST", assistant_path, Some(clear)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(error["error"], "validation_error");
        assert_org_route(&f, &org, &id, Some(&f.old), priority, &original).await;
        edit(
            &router,
            &token,
            &key_path,
            json!({"node_id":"", "endpoint_url":"https://127.0.0.1"}),
        )
        .await;
    } else {
        edit(&router, &token, assistant_path, clear).await;
    }
    assert_org_route(&f, &org, &id, None, priority, &original).await;
}

// Exercise the slug handler through its local node socket. Only the node can supply this reply.
async fn dispatch_org_slug(f: &Routes, org: &str) {
    use crate::services::node_ws_manager::{NodeOutboundMessage, NodeProxyResponse};
    let state = test_app_state(f.db.clone());
    set_test_approval_preference(&f.db, &f.actor, false).await;
    set_test_approval_preference(&f.db, org, false).await;
    crate::services::node_service::rotate_auth_token(
        &f.db,
        &state.encryption_keys,
        &f.actor,
        &f.old,
    )
    .await
    .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    register_test_node_connection(&state, &f.old, tx).await;
    let fallbacks = crate::services::node_routing_service::list_dispatchable_binding_node_ids(
        &f.db,
        org,
        &f.catalog,
        &state.node_ws_manager,
    )
    .await
    .unwrap();
    assert!(fallbacks.is_empty());
    let targets = crate::services::node_fanout_resolver::resolve_credential_fan_out_targets(
        &f.db, &f.actor, org, &f.catalog,
    )
    .await
    .unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].node_id, f.old);
    let token = access_token_with_scope(&state, &f.actor, "proxy");
    let (_, private) = crate::routes::build_router_with_state(state.clone());
    let result = request(
        private.with_state(state.clone()),
        &token,
        "GET",
        "/api/v1/proxy/s/org-route/check",
        None,
    );
    tokio::pin!(result);
    let outbound = tokio::select! {
        early = &mut result => panic!("slug request finished before dispatch: {early:?}"),
        msg = rx.recv() => msg.unwrap(),
    };
    let NodeOutboundMessage::Text(data) = outbound else {
        panic!("expected a proxy request");
    };
    let message: Value = serde_json::from_str(&data).unwrap();
    assert_eq!(message["service_slug"], "org-route");
    state.node_ws_manager.deliver_proxy_response(
        &f.old,
        NodeProxyResponse {
            request_id: message["request_id"].as_str().unwrap().to_owned(),
            status: 200,
            headers: vec![],
            body: br#"{"handled":"personal-node"}"#.to_vec(),
        },
    );
    let (status, response) = result.await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(response["handled"], "personal-node");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_personal_and_org_creations_share_one_binding() {
    let f = Routes::seed(false).await;
    let org = organization(&f).await;
    let (router, token) = f.router(f.db.clone());
    // Create journal collections before overlapping writes to two distinct services.
    create_connection(router.clone(), &token, Some(&org), "warmup", None).await;
    // Pause the org insert in its journal transaction while the personal request commits.
    let (monitor, arrived, resume) = pause_before_write(USER_SERVICES, &["insert"]);
    let db = observed(&f.db, "org-route-create", monitor).await;
    let (other, bearer) = f.router(db);
    let node = f.new.clone();
    let owner = org.clone();
    let mut org_request = tokio::spawn(async move {
        create_connection(other, &bearer, Some(&owner), "org-create", Some(&node)).await
    });
    tokio::select! {
        early = &mut org_request => panic!("org request finished before service insert: {early:?}"),
        ready = arrived => ready.unwrap(),
    }
    let personal = create_connection(router, &token, None, "personal-create", Some(&f.new)).await;
    let binding = sole_binding(&f, &f.new).await;
    assert_eq!(binding.get_str("user_id").unwrap(), f.actor);
    assert!(binding.get_bool("is_active").unwrap());
    resume.send(()).unwrap();
    let other = org_request.await.unwrap();
    for (owner, id) in [(&f.actor, personal), (&org, other)] {
        let service: UserService =
            f.db.collection(USER_SERVICES)
                .find_one(doc! {"_id":id, "user_id":owner, "is_active":true})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(service.node_id.as_deref(), Some(f.new.as_str()));
    }
    assert_eq!(
        sole_binding(&f, &f.new).await.as_bytes(),
        binding.as_bytes()
    );
}

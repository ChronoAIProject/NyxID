//! Real chat dispatch tests for the stored-catalog attachment capability.
use super::*;
use crate::models::{
    downstream_service::{self, DownstreamService},
    service_endpoint::{self, PublicationState, ServiceEndpoint},
};
use crate::services::{
    assistant_authority_tests::{Fixture, connected, orchestrator_fixture},
    assistant_upload_service as uploads, catalog_spec_registry, catalog_spec_sync,
    lark_attachment_tool::{self as attachment, UPLOAD_OPERATION},
    service_endpoint_service,
};
use mongodb::bson::{self, doc};
use serde_json::{Value, json};
use uuid::Uuid;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

struct BaseFixture {
    f: Fixture,
    server: MockServer,
    catalog: DownstreamService,
    instance: String,
    endpoint: ServiceEndpoint,
    attachment_id: String,
}

struct PausedTokenExchange {
    endpoint: String,
    reached: std::sync::Arc<tokio::sync::Notify>,
    release: std::sync::Arc<tokio::sync::Notify>,
    task: tokio::task::JoinHandle<()>,
}

impl PausedTokenExchange {
    async fn start(fail_create_preparation: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let reached = std::sync::Arc::new(tokio::sync::Notify::new());
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        let signal = reached.clone();
        let resume = release.clone();
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let app = axum::Router::new().route(
            "/token",
            axum::routing::post(move || {
                let count = count.clone();
                let signal = signal.clone();
                let resume = resume.clone();
                async move {
                    if count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 2 {
                        signal.notify_one();
                        resume.notified().await;
                        if fail_create_preparation {
                            return (
                                axum::http::StatusCode::BAD_GATEWAY,
                                axum::Json(json!({"error":"exchange unavailable"})),
                            );
                        }
                    }
                    (
                        axum::http::StatusCode::OK,
                        axum::Json(json!({"token":"tenant-test-token","expires_in":60})),
                    )
                }
            }),
        );
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            endpoint,
            reached,
            release,
            task,
        }
    }
}

impl Drop for PausedTokenExchange {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn flag(f: &Fixture, key: &str, enabled: bool) {
    let now = bson::DateTime::now();
    f.state
        .db
        .collection::<bson::Document>("feature_flag_overrides")
        .update_one(
            doc! {"flag_key":key,"target_kind":"user","target_key":&f.owner},
            doc! {"$set":{"enabled":enabled,"updated_at":now},"$setOnInsert":{
            "_id":Uuid::new_v4().to_string(),"org_user_id":bson::Bson::Null,
            "updated_by":"test","created_at":now}},
        )
        .upsert(true)
        .await
        .unwrap();
}
async fn writes(f: &Fixture, enabled: bool) {
    flag(
        f,
        crate::services::feature_flag_service::ASSISTANT_ATTACHMENT_WRITES_FLAG_KEY,
        enabled,
    )
    .await;
}
async fn bind_upload(f: &Fixture, bytes: &[u8]) -> String {
    let item = uploads::upload(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
        "receipt.txt",
        bytes.to_vec(),
    )
    .await
    .unwrap();
    let message = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .find_one(doc! {"conversation_id":&f.row.id,"role":"user"})
        .await
        .unwrap()
        .unwrap();
    let mut session = f.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    uploads::bind(
        &f.state.db,
        &f.owner,
        &f.row.id,
        message.get_str("_id").unwrap(),
        std::slice::from_ref(&item.id),
        &mut session,
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    item.id
}

async fn bump_credential_epoch(f: &Fixture, credential: &str, previous: i64) {
    let credentials = f.state.db.collection::<bson::Document>("user_api_keys");
    // Legacy credentials omit the stored field but deserialize with epoch 1.
    let changed = credentials
        .update_one(
            doc! {"_id":credential},
            vec![doc! {
                "$set":{"credential_epoch":{"$add":[{"$ifNull":["$credential_epoch",1i64]},1i64]}}
            }],
        )
        .await
        .unwrap();
    assert_eq!(changed.matched_count, 1);
    let stored = credentials
        .find_one(doc! {"_id":credential})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.get_i64("credential_epoch").unwrap(), previous + 1);
}

fn setup(name: &'static str) -> futures::future::BoxFuture<'static, BaseFixture> {
    Box::pin(async move {
        let f = orchestrator_fixture(name).await;
        crate::db::ensure_lark_base_write_indexes(&f.state.db)
            .await
            .unwrap();
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/open-apis/drive/v1/medias/upload_all"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":0,"data":{"file_token":"file_receipt"}})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/open-apis/bitable/v1/apps/base/tables/table/fields"))
            .respond_with(ResponseTemplate::new(200).set_body_json(schema(false)))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/open-apis/bitable/v1/apps/base/tables/table/records"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":0,"data":{"record":{"record_id":"rec_receipt"}}})),
            )
            .mount(&server)
            .await;
        let mut catalog = downstream_service::test_helpers::dummy_service();
        catalog.id = Uuid::new_v4().to_string();
        catalog.name = "Lark".into();
        catalog.slug = "api-lark".into();
        catalog.created_by = "system".into();
        let base_url = format!("{}/open-apis", server.uri());
        catalog.base_url = base_url.clone();
        f.state
            .db
            .collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
            .insert_one(&catalog)
            .await
            .unwrap();
        let instance = connected(&f.state.db, &f.owner, "receipts", &base_url).await;
        f.state
            .db
            .collection::<bson::Document>(crate::models::user_service::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&instance},
                doc! {"$set":{"catalog_service_id":&catalog.id}},
            )
            .await
            .unwrap();
        let credential = Uuid::new_v4().to_string();
        let encrypted = f
            .state
            .encryption_keys
            .encrypt(b"owner-oauth-test-token")
            .await
            .unwrap();
        f.state.db.collection::<bson::Document>("user_api_keys").insert_one(doc! {
            "_id":&credential,"user_id":&f.owner,"label":"Receipt account","credential_type":"oauth2","status":"active",
            "access_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},
            "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
        }).await.unwrap();
        f.state
            .db
            .collection::<bson::Document>("user_services")
            .update_one(
                doc! {"_id":&instance},
                doc! {"$set":{"api_key_id":credential,"auth_method":"bearer"}},
            )
            .await
            .unwrap();
        let connection = f
            .state
            .db
            .collection::<bson::Document>("user_services")
            .find_one(doc! {"_id":&instance})
            .await
            .unwrap()
            .unwrap();
        f.state
            .db
            .collection::<bson::Document>("user_endpoints")
            .update_one(
                doc! {"_id":connection.get_str("endpoint_id").unwrap()},
                doc! {"$set":{"label":"Receipt account"}},
            )
            .await
            .unwrap();
        let spec = catalog_spec_registry::spec_for_slug(&catalog.slug).unwrap();
        let mut endpoint = None;
        for input in catalog_spec_sync::destination_endpoint_inputs(&catalog, &spec).unwrap() {
            if !matches!(
                input.name.as_str(),
                UPLOAD_OPERATION
                    | "bitable_record_create"
                    | "bitable_fields_list"
                    | "bitable_record_create_with_attachments"
            ) {
                continue;
            }
            let row = service_endpoint_service::endpoint_from_input(
                &catalog.id,
                input,
                PublicationState::Published,
            );
            f.state
                .db
                .collection::<ServiceEndpoint>(service_endpoint::COLLECTION_NAME)
                .insert_one(&row)
                .await
                .unwrap();
            if row.name == "bitable_record_create_with_attachments" {
                endpoint = Some(row);
            }
        }
        writes(&f, true).await;
        let attachment_id = bind_upload(&f, b"receipt-a bytes").await;
        BaseFixture {
            f,
            server,
            catalog,
            instance,
            endpoint: endpoint.unwrap(),
            attachment_id,
        }
    })
}
async fn use_paused_tenant_exchange(u: &BaseFixture, exchange: &PausedTokenExchange) -> String {
    let instance =
        u.f.state
            .db
            .collection::<bson::Document>("user_services")
            .find_one(doc! {"_id":&u.instance})
            .await
            .unwrap()
            .unwrap();
    let credential = instance.get_str("api_key_id").unwrap().to_owned();
    let encrypted = u.f.state.encryption_keys.encrypt(b"{}").await.unwrap();
    u.f.state.db.collection::<bson::Document>("user_api_keys").update_one(doc! {"_id":&credential},doc! {"$set":{
        "credential_type":"api_key","credential_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted}}}).await.unwrap();
    u.f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":&u.instance},
            doc! {"$set":{"auth_method":"token_exchange"}},
        )
        .await
        .unwrap();
    let config = crate::models::downstream_service::TokenExchangeConfig {
        endpoint: exchange.endpoint.clone(),
        request_encoding: "json".into(),
        request_template: json!({}),
        token_response_path: "token".into(),
        ttl_response_path: Some("expires_in".into()),
        default_ttl_secs: 1,
        injection: "bearer".into(),
        error_code_path: None,
        error_message_path: None,
        credential_fields: vec![],
    };
    u.f.state
        .db
        .collection::<bson::Document>(downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&u.catalog.id},
            doc! {"$set":{"token_exchange_config":bson::to_bson(&config).unwrap()}},
        )
        .await
        .unwrap();
    credential
}

async fn authenticate(f: &Fixture) -> McpAuthContext {
    let key = crate::services::assistant_agent_credential_service::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
    )
    .await
    .unwrap()
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-api-key", key.raw_key.parse().unwrap());
    authenticate_mcp(&f.state, &headers, false).await.unwrap()
}
async fn rpc(f: &Fixture, auth: &McpAuthContext, method: &str, params: Option<Value>) -> Value {
    let request = JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: method.into(),
        params,
    };
    let response = if method == "tools/list" {
        Box::pin(handle_tools_list(&f.state, auth, None, &request)).await
    } else {
        Box::pin(handle_tools_call(
            &f.state,
            auth,
            None,
            &request,
            false,
            crate::services::billing::route_inventory::internal_node_dispatch_permit(),
        ))
        .await
    };
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap(),
    )
    .unwrap()
}
async fn call(u: &BaseFixture, auth: &McpAuthContext, meta: bool, args: Value) -> Value {
    let name = "receipts__bitable_record_create_with_attachments";
    let params = if meta {
        json!({"name":"nyx__call_tool","arguments":{"tool_name":name,"arguments_json":args.to_string()}})
    } else {
        json!({"name":name,"arguments":args})
    };
    rpc(&u.f, auth, "tools/call", Some(params)).await
}
fn schema(more: bool) -> Value {
    json!({"code":0,"data":{"has_more":more,"page_token":"next","items":[
        {"field_name":"Receipt","type":17},{"field_name":"Amount","type":2},
        {"field_name":"Type","type":3,"property":{"options":[{"name":"Meal"}]}},
        {"field_name":"Card Number","type":3,"property":{"options":[{"name":"1234"}]}},
        {"field_name":"Date","type":5}]}})
}
fn args(id: &str) -> Value {
    json!({"app_token":"base","table_id":"table","fields":{"Amount":12.34,"Type":"Meal","Card Number":"1234","Date":1700000000000i64},"attachments":{"Receipt":[id]}})
}
fn text(body: &Value) -> &str {
    body["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("{body}"))
}
fn payload(body: &Value) -> Value {
    serde_json::from_str(text(body)).unwrap_or_else(|error| panic!("{error}: {body}"))
}
fn assert_error(body: &Value) {
    assert_eq!(body["result"]["isError"], true, "{body}");
}
async fn loaded(u: &BaseFixture, auth: &McpAuthContext) -> mcp_service::McpToolService {
    load_all_services_for_meta_tools(&u.f.state, auth)
        .await
        .unwrap()
        .into_iter()
        .find(|service| service.service_id == u.instance)
        .unwrap()
}
async fn ledger(u: &BaseFixture) -> crate::models::lark_base_write::LarkBaseWrite {
    u.f.state
        .db
        .collection(crate::models::lark_base_write::COLLECTION_NAME)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap()
}
async fn requests(u: &BaseFixture) -> Vec<wiremock::Request> {
    u.server.received_requests().await.unwrap()
}
async fn assert_no_create(u: &BaseFixture) {
    assert!(
        requests(u)
            .await
            .iter()
            .all(|request| !request.url.path().ends_with("/records"))
    );
}
async fn cleanup(u: BaseFixture) {
    u.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lark_base_write_direct_and_meta_upload_all_then_create_with_query_token_and_file_token_cells()
 {
    for meta in [false, true] {
        let u = setup("base_happy").await;
        let auth = authenticate(&u.f).await;
        let service = loaded(&u, &auth).await;
        let endpoint = service
            .endpoints
            .iter()
            .find(|endpoint| endpoint.name == "bitable_record_create_with_attachments")
            .unwrap();
        assert!(matches!(
            endpoint.attachment_input,
            Some(attachment::Projection::Managed(_))
        ));
        let response = call(&u, &auth, meta, args(&u.attachment_id)).await;
        assert_eq!(
            payload(&response),
            json!({"record_id":"rec_receipt","already_created":false}),
            "{response}"
        );
        let sent = requests(&u).await;
        assert_eq!(sent.len(), 3);
        assert_eq!(sent[0].method.as_str(), "GET");
        assert!(sent[1].url.path().ends_with("upload_all"));
        let create = &sent[2];
        let token = create
            .url
            .query_pairs()
            .find(|(name, _)| name == "client_token")
            .unwrap()
            .1
            .to_string();
        assert_eq!(Uuid::parse_str(&token).unwrap().get_version_num(), 4);
        let body: Value = serde_json::from_slice(&create.body).unwrap();
        assert_eq!(
            body["fields"]["Receipt"],
            json!([{"file_token":"file_receipt"}])
        );
        assert_eq!(body["fields"]["Amount"], 12.34);
        assert!(body.get("client_token").is_none());
        assert!(!String::from_utf8_lossy(&create.body).contains(&u.attachment_id));
        let row = ledger(&u).await;
        assert_eq!(row.client_token, token);
        assert_eq!(
            row.state,
            crate::models::lark_base_write::WriteState::Succeeded
        );
        assert_eq!(row.contract_digests.len(), 4);
        let repeated = call(&u, &auth, meta, args(&u.attachment_id)).await;
        assert_eq!(payload(&repeated)["already_created"], true);
        assert_eq!(requests(&u).await.len(), 3);
        cleanup(u).await;
    }
}

#[tokio::test]
async fn lark_base_write_new_upload_same_bytes_replays_after_original_deletion_and_changed_fields_or_assignment_refuse_without_provider()
 {
    let u = setup("base_replay_new_id").await;
    let auth = authenticate(&u.f).await;
    assert_eq!(
        payload(&call(&u, &auth, false, args(&u.attachment_id)).await)["record_id"],
        "rec_receipt"
    );
    let new = bind_upload(&u.f, b"receipt-a bytes").await;
    u.f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
        .delete_one(doc! {"_id":&u.attachment_id})
        .await
        .unwrap();
    let replay = call(&u, &auth, true, args(&new)).await;
    assert_eq!(payload(&replay)["already_created"], true);
    let mut changed = args(&new);
    changed["fields"]["Amount"] = json!(42.34);
    assert_error(&call(&u, &auth, false, changed).await);
    let mut changed = args(&new);
    changed["attachments"] = json!({"Other Photo":[new]});
    assert_error(&call(&u, &auth, true, changed).await);
    assert_eq!(requests(&u).await.len(), 3);
    cleanup(u).await;
}

#[tokio::test]
async fn lark_base_write_success_replay_after_credential_rotation_makes_no_provider_calls() {
    let u = setup("base_replay_rotation").await;
    let auth = authenticate(&u.f).await;
    assert_eq!(
        payload(&call(&u, &auth, false, args(&u.attachment_id)).await)["record_id"],
        "rec_receipt"
    );
    let before = ledger(&u).await;
    assert_eq!(requests(&u).await.len(), 3);
    bump_credential_epoch(
        &u.f,
        &before.principal.credential_id,
        before.principal.epoch,
    )
    .await;
    let fresh_auth = authenticate(&u.f).await;
    let replay = call(&u, &fresh_auth, true, args(&u.attachment_id)).await;
    assert_eq!(payload(&replay)["record_id"], "rec_receipt", "{replay}");
    assert_eq!(payload(&replay)["already_created"], true, "{replay}");
    assert_eq!(requests(&u).await.len(), 3);
    let after = ledger(&u).await;
    assert_eq!(after.attempt, before.attempt);
    assert_eq!(after.principal, before.principal);
    assert_eq!(after.client_token, before.client_token);
    assert_eq!(
        bson::to_bson(&after.files).unwrap(),
        bson::to_bson(&before.files).unwrap()
    );
    assert_eq!(
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::lark_base_write_photo::COLLECTION_NAME)
            .count_documents(doc! {"write_id":&after.id,"attempt":&before.attempt})
            .await
            .unwrap(),
        1
    );
    cleanup(u).await;
}

#[tokio::test]
async fn lark_base_write_page_two_resolves_photo_and_invalid_loop_or_malformed_page_tokens_fail_closed()
 {
    use wiremock::matchers::query_param;
    for variant in ["second", "loop", "malformed", "exhausted"] {
        let u = setup("base_pages").await;
        let auth = authenticate(&u.f).await;
        let first = json!({"code":0,"data":{"has_more":true,"page_token":if variant=="malformed"{"../bad"}else{"page2"},"items":[{"field_name":"Amount","type":2}]}});
        Mock::given(method("GET"))
            .and(path("/open-apis/bitable/v1/apps/base/tables/table/fields"))
            .respond_with(ResponseTemplate::new(200).set_body_json(first))
            .with_priority(1)
            .up_to_n_times(1)
            .mount(&u.server)
            .await;
        if variant == "second" {
            Mock::given(method("GET")).and(path("/open-apis/bitable/v1/apps/base/tables/table/fields")).and(query_param("page_token","page2"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"code":0,"data":{"has_more":false,"items":schema(false)["data"]["items"].as_array().unwrap().iter().filter(|field|field["field_name"]!="Amount").cloned().collect::<Vec<_>>()}})))
                .with_priority(1).mount(&u.server).await;
        } else if variant == "loop" {
            Mock::given(method("GET"))
                .and(query_param("page_token", "page2"))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    json!({"code":0,"data":{"has_more":true,"page_token":"page2","items":[]}}),
                ))
                .with_priority(1)
                .mount(&u.server)
                .await;
        } else if variant == "exhausted" {
            let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(3));
            Mock::given(method("GET")).respond_with(move |_:&wiremock::Request| {
                let page=count.fetch_add(1,std::sync::atomic::Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_json(json!({"code":0,"data":{"has_more":true,"page_token":format!("page{page}"),"items":[]}}))
            }).with_priority(2).mount(&u.server).await;
        }
        let response = call(&u, &auth, false, args(&u.attachment_id)).await;
        if variant == "second" {
            assert_eq!(payload(&response)["record_id"], "rec_receipt");
            assert_eq!(requests(&u).await.len(), 4);
        } else {
            assert_error(&response);
            assert_no_create(&u).await;
            assert!(
                requests(&u)
                    .await
                    .iter()
                    .all(|request| request.method.as_str() == "GET")
            );
        }
        cleanup(u).await;
    }
}

#[tokio::test]
async fn lark_base_write_second_upload_http_200_error_releases_attempt_and_never_creates_partial_record()
 {
    let u = setup("base_upload_failure").await;
    let auth = authenticate(&u.f).await;
    let second = bind_upload(&u.f, b"receipt-b bytes").await;
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let sent = count.clone();
    Mock::given(method("POST"))
        .and(path("/open-apis/drive/v1/medias/upload_all"))
        .respond_with(move |_: &wiremock::Request| {
            let index = sent.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(if index == 0 {
                json!({"code":0,"data":{"file_token":"first"}})
            } else {
                json!({"code":999,"msg":"secret-provider-prose"})
            })
        })
        .with_priority(1)
        .mount(&u.server)
        .await;
    let mut input = args(&u.attachment_id);
    input["attachments"]["Receipt"] = json!([u.attachment_id, second]);
    let response = call(&u, &auth, false, input).await;
    assert_error(&response);
    assert!(!text(&response).contains("secret-provider-prose"));
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_no_create(&u).await;
    assert_eq!(
        ledger(&u).await.state,
        crate::models::lark_base_write::WriteState::FailedPreDispatch
    );
    assert_eq!(
        u.f.state
            .db
            .collection::<bson::Document>(crate::models::lark_base_write_photo::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    cleanup(u).await;
}

#[tokio::test]
async fn lark_base_write_upload_busy_codes_have_three_bounded_retries_and_a_successful_retry_creates_once()
 {
    for succeeds in [false, true] {
        let u = setup("base_upload_retry").await;
        let auth = authenticate(&u.f).await;
        let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let sent = count.clone();
        Mock::given(method("POST"))
            .and(path("/open-apis/drive/v1/medias/upload_all"))
            .respond_with(move |_: &wiremock::Request| {
                let index = sent.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_json(if succeeds && index == 2 {
                    json!({"code":0,"data":{"file_token":"retry-token"}})
                } else {
                    json!({"code":if index.is_multiple_of(2){1061045}else{1061006}})
                })
            })
            .with_priority(1)
            .mount(&u.server)
            .await;
        let response = call(&u, &auth, false, args(&u.attachment_id)).await;
        if succeeds {
            assert_eq!(payload(&response)["record_id"], "rec_receipt");
            assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 3);
        } else {
            assert_error(&response);
            assert_no_create(&u).await;
            assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 4);
        }
        cleanup(u).await;
    }
}

#[tokio::test]
async fn lark_base_write_field_read_permission_error_identifies_outbound_service_and_connection_without_provider_prose()
 {
    let u = setup("base_field_permission").await;
    let auth = authenticate(&u.f).await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"code":99991672,"msg":"secret-provider-prose"})),
        )
        .with_priority(1)
        .mount(&u.server)
        .await;
    let response = call(&u, &auth, false, args(&u.attachment_id)).await;
    assert_error(&response);
    assert!(text(&response).contains("receipts"));
    assert!(text(&response).contains("connection Receipt account"));
    assert!(!text(&response).contains("secret-provider-prose"));
    assert_no_create(&u).await;
    assert_eq!(requests(&u).await.len(), 1);
    cleanup(u).await;
}

#[tokio::test]
async fn lark_base_write_source_completeness_legacy_hash_duplicate_content_and_size_limits_refuse_locally()
 {
    for variant in [
        "source_missing",
        "source_omitted",
        "same_content",
        "oversized",
        "aggregate",
    ] {
        let u = setup("base_local_limits").await;
        let auth = authenticate(&u.f).await;
        let mut input = args(&u.attachment_id);
        if variant == "same_content" {
            let repeated = bind_upload(&u.f, b"receipt-a bytes").await;
            input["attachments"]["Receipt"] = json!([u.attachment_id, repeated]);
        }
        if variant.starts_with("source_") {
            u.f.state.db.collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME).update_one(doc! {"_id":&u.attachment_id},doc! {"$set":{"source":{"event_key":"event","declared":2i64,"index":0i64,"embedded":false,"turn_id":u.f.chat.turn_id.as_deref()}}}).await.unwrap();
            if variant == "source_omitted" {
                let second = bind_upload(&u.f, b"receipt-b bytes").await;
                u.f.state.db.collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME).update_one(doc! {"_id":second},doc! {"$set":{"source":{"event_key":"event","declared":2i64,"index":1i64,"embedded":false,"turn_id":u.f.chat.turn_id.as_deref()}}}).await.unwrap();
            }
        }
        if variant == "oversized" {
            u.f.state
                .db
                .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&u.attachment_id},
                    doc! {"$set":{"size":21i64*1024*1024}},
                )
                .await
                .unwrap();
        }
        if variant == "aggregate" {
            let b = bind_upload(&u.f, b"receipt-b bytes").await;
            let c = bind_upload(&u.f, b"receipt-c bytes").await;
            let ids = vec![u.attachment_id.clone(), b, c];
            u.f.state
                .db
                .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
                .update_many(
                    doc! {"_id":{"$in":&ids}},
                    doc! {"$set":{"size":18i64*1024*1024}},
                )
                .await
                .unwrap();
            input["attachments"]["Receipt"] = json!(ids);
        }
        let response = call(&u, &auth, false, input).await;
        assert_error(&response);
        assert!(requests(&u).await.is_empty());
        cleanup(u).await;
    }
    let u = setup("base_legacy_hash").await;
    let auth = authenticate(&u.f).await;
    u.f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&u.attachment_id},
            doc! {"$unset":{"content_sha256":""}},
        )
        .await
        .unwrap();
    assert_eq!(
        payload(&call(&u, &auth, false, args(&u.attachment_id)).await)["record_id"],
        "rec_receipt"
    );
    cleanup(u).await;
}

async fn wait_upload(u: &BaseFixture) {
    for _ in 0..200 {
        if requests(u)
            .await
            .iter()
            .any(|request| request.url.path().ends_with("upload_all"))
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("upload did not reach provider");
}

#[tokio::test]
async fn lark_base_write_guard_revocation_stop_credential_and_dependency_changes_between_uploads_block_every_further_send()
 {
    for change in [
        "stop",
        "key",
        "credential",
        "epoch",
        "override",
        "scope",
        "destination",
        "contract",
        "unpublish",
    ] {
        let u = setup("base_guard").await;
        let auth = authenticate(&u.f).await;
        let second = bind_upload(&u.f, b"receipt-b bytes").await;
        Mock::given(method("POST"))
            .and(path("/open-apis/drive/v1/medias/upload_all"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":0,"data":{"file_token":"first"}}))
                    .set_delay(std::time::Duration::from_millis(400)),
            )
            .with_priority(1)
            .mount(&u.server)
            .await;
        let mut input = args(&u.attachment_id);
        input["attachments"]["Receipt"] = json!([u.attachment_id, second]);
        let (response, ()) = tokio::join!(call(&u, &auth, false, input), async {
            wait_upload(&u).await;
            match change {
                "stop" => {
                    u.f.state
                        .db
                        .collection::<bson::Document>("assistant_conversations")
                        .update_one(
                            doc! {"_id":&u.f.row.id},
                            doc! {"$set":{"active_turn.stop_requested":true}},
                        )
                        .await
                        .unwrap();
                }
                "key" => {
                    u.f.state
                        .db
                        .collection::<bson::Document>("api_keys")
                        .update_one(
                            doc! {"_id":&u.f.chat.api_key_id},
                            doc! {"$set":{"is_active":false}},
                        )
                        .await
                        .unwrap();
                }
                "credential" => {
                    let instance =
                        u.f.state
                            .db
                            .collection::<bson::Document>("user_services")
                            .find_one(doc! {"_id":&u.instance})
                            .await
                            .unwrap()
                            .unwrap();
                    u.f.state
                        .db
                        .collection::<bson::Document>("user_api_keys")
                        .update_one(
                            doc! {"_id":instance.get_str("api_key_id").unwrap()},
                            doc! {"$set":{"status":"revoked"}},
                        )
                        .await
                        .unwrap();
                }
                "epoch" => {
                    let row =
                        u.f.state
                            .db
                            .collection::<bson::Document>("user_services")
                            .find_one(doc! {"_id":&u.instance})
                            .await
                            .unwrap()
                            .unwrap();
                    u.f.state
                        .db
                        .collection::<bson::Document>("user_api_keys")
                        .update_one(
                            doc! {"_id":row.get_str("api_key_id").unwrap()},
                            doc! {"$set":{"credential_epoch":2i64}},
                        )
                        .await
                        .unwrap();
                }
                "override" => {
                    let row =
                        u.f.state
                            .db
                            .collection::<bson::Document>("user_services")
                            .find_one(doc! {"_id":&u.instance})
                            .await
                            .unwrap()
                            .unwrap();
                    let mut credential =
                        u.f.state
                            .db
                            .collection::<bson::Document>("user_api_keys")
                            .find_one(doc! {"_id":row.get_str("api_key_id").unwrap()})
                            .await
                            .unwrap()
                            .unwrap();
                    let id = Uuid::new_v4().to_string();
                    credential.insert("_id", &id);
                    u.f.state
                        .db
                        .collection::<bson::Document>("user_api_keys")
                        .insert_one(credential)
                        .await
                        .unwrap();
                    u.f.state.db.collection::<bson::Document>("agent_service_bindings").insert_one(doc! {"_id":Uuid::new_v4().to_string(),"api_key_id":&u.f.chat.api_key_id,"user_service_id":&u.instance,"user_id":&u.f.owner,"user_api_key_id":id,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
                }
                "scope" => {
                    receipt_scope(&u).await;
                }
                "destination" => {
                    let row =
                        u.f.state
                            .db
                            .collection::<bson::Document>("user_services")
                            .find_one(doc! {"_id":&u.instance})
                            .await
                            .unwrap()
                            .unwrap();
                    u.f.state
                        .db
                        .collection::<bson::Document>("user_endpoints")
                        .update_one(
                            doc! {"_id":row.get_str("endpoint_id").unwrap()},
                            doc! {"$set":{"url":format!("{}/other",u.server.uri())}},
                        )
                        .await
                        .unwrap();
                }
                _ => {
                    u.f.state
                        .db
                        .collection::<bson::Document>(service_endpoint::COLLECTION_NAME)
                        .update_one(
                            doc! {"service_id":&u.catalog.id,"name":"bitable_record_create"},
                            if change == "unpublish" {
                                doc! {"$set":{"publication":"draft"}}
                            } else {
                                doc! {"$set":{"path":"/different"}}
                            },
                        )
                        .await
                        .unwrap();
                }
            }
        });
        assert_error(&response);
        assert_no_create(&u).await;
        assert_eq!(
            requests(&u)
                .await
                .iter()
                .filter(|request| request.url.path().ends_with("upload_all"))
                .count(),
            1,
            "{change}"
        );
        cleanup(u).await;
    }
}

#[tokio::test]
async fn lark_base_write_four_flag_combinations_both_dispatch_paths_and_discovery_keep_plain_create_unchanged()
 {
    for media in [false, true] {
        for writes_on in [false, true] {
            for meta in [false, true] {
                let u = setup("base_flag_matrix").await;
                flag(
                    &u.f,
                    crate::services::feature_flag_service::ASSISTANT_CHANNEL_MEDIA_FLAG_KEY,
                    media,
                )
                .await;
                writes(&u.f, writes_on).await;
                let auth = authenticate(&u.f).await;
                let listed = rpc(&u.f, &auth, "tools/list", None).await.to_string();
                assert_eq!(
                    listed.contains("receipts__bitable_record_create_with_attachments"),
                    writes_on,
                    "{listed}"
                );
                assert!(listed.contains("\"name\":\"receipts__bitable_record_create\""));
                assert!(listed.contains("receipts__bitable_fields_list"));
                let response = call(&u, &auth, meta, args(&u.attachment_id)).await;
                if writes_on {
                    assert_eq!(payload(&response)["record_id"], "rec_receipt");
                } else {
                    assert_error(&response);
                    assert!(requests(&u).await.is_empty());
                }
                let before = requests(&u).await.len();
                let plain =
                    json!({"app_token":"base","table_id":"table","fields":{"Amount":12.34}});
                let params = if meta {
                    json!({"name":"nyx__call_tool","arguments":{"tool_name":"receipts__bitable_record_create","arguments_json":plain.to_string()}})
                } else {
                    json!({"name":"receipts__bitable_record_create","arguments":plain})
                };
                let response = rpc(&u.f, &auth, "tools/call", Some(params)).await;
                assert_eq!(payload(&response)["code"], 0, "{response}");
                let sent = requests(&u).await;
                assert_eq!(sent.len(), before + 1);
                assert_eq!(
                    serde_json::from_slice::<Value>(&sent.last().unwrap().body).unwrap(),
                    json!({"fields":{"Amount":12.34}})
                );
                cleanup(u).await;
            }
        }
    }
}

#[tokio::test]
async fn lark_base_write_config_exact_descriptors_nonchat_and_async_exclude_attachment_operations_locally()
 {
    let u = setup("base_surfaces").await;
    let auth = authenticate(&u.f).await;
    let service = loaded(&u, &auth).await;
    let endpoint = service
        .endpoints
        .iter()
        .find(|ep| ep.name == attachment::COMPOSITE_OPERATION)
        .unwrap();
    assert!(
        mcp_service::prepare_proxy_tool_call(&service, endpoint, &args(&u.attachment_id)).is_err()
    );
    assert!(
        mcp_service::prepare_exact_proxy_tool_call(
            &service,
            endpoint,
            &args(&u.attachment_id),
            Some("exact")
        )
        .is_err()
    );
    assert!(
        mcp_service::build_mcp_operation_descriptor(&service, endpoint, &args(&u.attachment_id))
            .is_err()
    );
    assert!(
        mcp_service::prepare_chat_proxy_tool_call(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            None,
            &service,
            endpoint,
            &args(&u.attachment_id)
        )
        .await
        .is_err()
    );
    let mut asynchronous = endpoint.clone();
    asynchronous.async_operation=Some(serde_json::from_value(json!({"status_operation":"status","result_operation":"result","cancel_operation":"cancel","id_field":"id","id_parameter":"id","status_field":"status","success_states":["done"],"failure_states":["error"],"error_field":null})).unwrap());
    assert!(
        mcp_service::prepare_chat_proxy_tool_call(
            &u.f.state.db,
            &u.f.state.encryption_keys,
            auth.chat.as_ref(),
            &service,
            &asynchronous,
            &args(&u.attachment_id)
        )
        .await
        .is_err()
    );
    let exact = serde_json::to_value(mcp_service::exact_operation_view(std::slice::from_ref(
        &service,
    )))
    .unwrap()
    .to_string();
    assert!(!exact.contains(attachment::COMPOSITE_OPERATION));
    assert!(!exact.contains(UPLOAD_OPERATION));
    assert!(exact.contains("bitable_record_create"));
    let config = crate::handlers::mcp::get_mcp_config(
        axum::extract::State(u.f.state.clone()),
        u.f.auth.clone(),
    )
    .await
    .unwrap();
    let config = serde_json::to_value(config.0).unwrap().to_string();
    assert!(!config.contains(attachment::COMPOSITE_OPERATION));
    assert!(!config.contains(UPLOAD_OPERATION));
    assert!(config.contains("bitable_record_create"));
    assert!(requests(&u).await.is_empty());
    cleanup(u).await;
}

#[tokio::test]
async fn lark_base_write_uncertain_replay_never_calls_provider_and_definite_correction_uses_new_client_token()
 {
    for code in [1254001, 1254607, 1254291, 1255006, 1254608, 999999] {
        let u = setup("base_create_outcome").await;
        let auth = authenticate(&u.f).await;
        Mock::given(method("POST"))
            .and(path("/open-apis/bitable/v1/apps/base/tables/table/records"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"code":code})))
            .with_priority(1)
            .up_to_n_times(1)
            .expect(1)
            .mount(&u.server)
            .await;
        assert_error(&call(&u, &auth, false, args(&u.attachment_id)).await);
        let old = ledger(&u).await;
        assert_eq!(
            old.state,
            if code == 1254001 {
                crate::models::lark_base_write::WriteState::FailedDefinite
            } else {
                crate::models::lark_base_write::WriteState::Uncertain
            }
        );
        let mut correction = args(&u.attachment_id);
        correction["fields"]["Amount"] = json!(13.34);
        let response = call(&u, &auth, true, correction).await;
        if code == 1254001 {
            assert_eq!(payload(&response)["record_id"], "rec_receipt");
            assert_ne!(ledger(&u).await.client_token, old.client_token);
            assert_eq!(
                requests(&u)
                    .await
                    .iter()
                    .filter(|request| request.url.path().ends_with("/records"))
                    .count(),
                2
            );
        } else {
            assert_error(&response);
            assert_eq!(requests(&u).await.len(), 3);
        }
        cleanup(u).await;
    }
}

#[tokio::test]
async fn lark_base_write_tenant_token_exchange_and_oauth_user_principals_use_existing_transport_injection()
 {
    for tenant in [false, true] {
        let u = setup("base_principals").await;
        if tenant {
            Mock::given(path("/token"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"token":"tenant-test-token","expires_in":60})),
                )
                .mount(&u.server)
                .await;
            let instance =
                u.f.state
                    .db
                    .collection::<bson::Document>("user_services")
                    .find_one(doc! {"_id":&u.instance})
                    .await
                    .unwrap()
                    .unwrap();
            let encrypted = u.f.state.encryption_keys.encrypt(b"{}").await.unwrap();
            u.f.state.db.collection::<bson::Document>("user_api_keys").update_one(doc! {"_id":instance.get_str("api_key_id").unwrap()},doc! {"$set":{
                "credential_type":"api_key","credential_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted}}}).await.unwrap();
            u.f.state
                .db
                .collection::<bson::Document>("user_services")
                .update_one(
                    doc! {"_id":&u.instance},
                    doc! {"$set":{"auth_method":"token_exchange"}},
                )
                .await
                .unwrap();
            let config = crate::models::downstream_service::TokenExchangeConfig {
                endpoint: format!("{}/token", u.server.uri()),
                request_encoding: "json".into(),
                request_template: json!({}),
                token_response_path: "token".into(),
                ttl_response_path: Some("expires_in".into()),
                default_ttl_secs: 60,
                injection: "bearer".into(),
                error_code_path: None,
                error_message_path: None,
                credential_fields: vec![],
            };
            u.f.state
                .db
                .collection::<bson::Document>(downstream_service::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&u.catalog.id},
                    doc! {"$set":{"token_exchange_config":bson::to_bson(&config).unwrap()}},
                )
                .await
                .unwrap();
        }
        let auth = authenticate(&u.f).await;
        assert_eq!(
            payload(&call(&u, &auth, false, args(&u.attachment_id)).await)["record_id"],
            "rec_receipt"
        );
        for request in requests(&u)
            .await
            .iter()
            .filter(|request| request.url.path() != "/token")
        {
            let auth = request
                .headers
                .get("authorization")
                .unwrap()
                .to_str()
                .unwrap();
            assert_eq!(
                auth,
                if tenant {
                    "Bearer tenant-test-token"
                } else {
                    "Bearer owner-oauth-test-token"
                }
            );
        }
        cleanup(u).await;
    }
}

async fn receipt_scope(u: &BaseFixture) {
    let scope:crate::models::agent_operation_scope::AgentOperationScope=serde_json::from_value(json!({"revision":1,"catalog_service_id":u.catalog.id,"operations":[
        {"endpoint_id":u.endpoint.id,"rule":{"method":"POST","path_template":attachment::COMPOSITE_PATH},"inputs":{
            "path":{"app_token":{"type":"exact","value":"base"},"table_id":{"type":"exact","value":"table"}},
            "body":{"type":"object","required":["fields","attachments"],"properties":{
                "fields":{"type":"object","required":["Type","Card Number","Date","Amount"],"properties":{
                    "Type":{"type":"one_of","values":["Meal"]},"Card Number":{"type":"one_of","values":["1234"]},
                    "Date":{"type":"integer","minimum":1600000000000i64,"maximum":2000000000000i64},
                    "Amount":{"type":"number","minimum":0,"maximum":100000,"max_fraction_digits":2}}},
                "attachments":{"type":"object","required":["Receipt"],"properties":{"Receipt":{"type":"array","max_items":10,"items":{"type":"string","max_length":64}}}}
            }}}}]})).unwrap();
    let scopes =
        crate::models::agent_operation_scope::OperationScopes::from([(u.instance.clone(), scope)]);
    u.f.state
        .db
        .collection::<bson::Document>("api_keys")
        .update_one(
            doc! {"_id":&u.f.chat.api_key_id},
            doc! {"$set":{"assistant_operation_scopes":bson::to_bson(&scopes).unwrap()}},
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn lark_base_write_real_scope_evaluator_accepts_decimal_recipe_and_denies_missing_option_property_plain_create_and_upload()
 {
    let u = setup("base_recipe").await;
    receipt_scope(&u).await;
    let auth = authenticate(&u.f).await;
    for variant in [
        "missing",
        "option",
        "property",
        "empty_photo",
        "omitted_photo",
        "override",
        "wrong_table",
    ] {
        let mut input = args(&u.attachment_id);
        match variant {
            "missing" => {
                input["fields"].as_object_mut().unwrap().remove("Date");
            }
            "option" => input["fields"]["Type"] = json!("Unknown"),
            "property" => input["fields"]["Extra"] = json!("No"),
            "empty_photo" => input["attachments"]["Receipt"] = json!([]),
            "omitted_photo" => {
                input.as_object_mut().unwrap().remove("attachments");
            }
            "override" => input["allow_duplicate_photo"] = json!(true),
            _ => input["table_id"] = json!("other"),
        }
        assert_error(&call(&u, &auth, false, input).await);
    }
    for (name, arguments) in [
        (
            "receipts__bitable_record_create",
            json!({"app_token":"base","table_id":"table","fields":{"Amount":12.34}}),
        ),
        (
            "receipts__bitable_media_upload",
            json!({"attachment_id":u.attachment_id,"parent_type":"bitable_file","parent_node":"base"}),
        ),
    ] {
        assert_error(
            &rpc(
                &u.f,
                &auth,
                "tools/call",
                Some(json!({"name":name,"arguments":arguments})),
            )
            .await,
        );
    }
    assert!(requests(&u).await.is_empty());
    assert_eq!(
        payload(&call(&u, &auth, true, args(&u.attachment_id)).await)["record_id"],
        "rec_receipt"
    );
    cleanup(u).await;
}

async fn wait_path(u: &BaseFixture, suffix: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            if requests(u)
                .await
                .iter()
                .any(|r| r.url.path().ends_with(suffix))
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn lark_base_write_same_photo_distinct_conversations_refuses_stalled_dispatch_then_replays_one_create()
 {
    use crate::services::{
        assistant_acknowledgement_service as acks, assistant_authority_tests::new_orchestrator,
    };
    let u = setup("base_two_chats").await;
    let exchange = PausedTokenExchange::start(false).await;
    use_paused_tenant_exchange(&u, &exchange).await;
    let auth = authenticate(&u.f).await;
    let row = new_orchestrator(&u.f.state, &u.f.owner, "Another receipt").await;
    assert_ne!(row.id, u.f.row.id);
    let chat = acks::for_key(&u.f.state.db, &u.f.owner, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let f = Fixture {
        state: u.f.state.clone(),
        owner: u.f.owner.clone(),
        nyxbot_thread: row.id.clone(),
        row,
        chat,
        auth: u.f.auth.clone(),
    };
    let id = bind_upload(&f, b"receipt-a bytes").await;
    assert_ne!(id, u.attachment_id);
    let auth2 = authenticate(&f).await;
    let params =
        json!({"name":"receipts__bitable_record_create_with_attachments","arguments":args(&id)});
    let (first, second) = tokio::join!(call(&u, &auth, false, args(&u.attachment_id)), async {
        tokio::time::timeout(
            std::time::Duration::from_secs(20),
            exchange.reached.notified(),
        )
        .await
        .unwrap();
        assert_eq!(
            ledger(&u).await.state,
            crate::models::lark_base_write::WriteState::Dispatching
        );
        let response = rpc(&f, &auth2, "tools/call", Some(params.clone())).await;
        exchange.release.notify_one();
        response
    });
    assert_eq!(payload(&first)["record_id"], "rec_receipt");
    assert_error(&second);
    assert!(
        text(&second).contains("A Base write for this photo is in progress; try again shortly"),
        "{second}"
    );
    assert!(!text(&second).contains("outcome unknown"), "{second}");
    assert_eq!(requests(&u).await.len(), 3);
    let replay = rpc(&f, &auth2, "tools/call", Some(params)).await;
    assert_eq!(payload(&replay)["already_created"], true);
    assert_eq!(requests(&u).await.len(), 3);
    let b = bind_upload(&f, b"receipt-b bytes").await;
    let mut overlap = args(&id);
    overlap["attachments"]["Receipt"] = json!([id, b]);
    assert_error(&rpc(&f,&auth2,"tools/call",Some(json!({"name":"receipts__bitable_record_create_with_attachments","arguments":overlap}))).await);
    assert_eq!(requests(&u).await.len(), 3);
    cleanup(u).await;
}

#[tokio::test]
async fn lark_base_write_concurrent_overlapping_batches_dispatch_at_most_once() {
    let u = setup("base_overlap_dispatch").await;
    let auth = authenticate(&u.f).await;
    let b = bind_upload(&u.f, b"receipt-b bytes").await;
    let c = bind_upload(&u.f, b"receipt-c bytes").await;
    let mut left = args(&u.attachment_id);
    left["attachments"]["Receipt"] = json!([u.attachment_id, b]);
    let mut right = args(&u.attachment_id);
    right["attachments"]["Receipt"] = json!([u.attachment_id, c]);
    let (left, right) = tokio::join!(call(&u, &auth, false, left), call(&u, &auth, true, right));
    assert_eq!(
        usize::from(left["result"]["isError"] != true)
            + usize::from(right["result"]["isError"] != true),
        1,
        "{left} {right}"
    );
    assert_eq!(
        requests(&u)
            .await
            .iter()
            .filter(|r| r.url.path().ends_with("/records"))
            .count(),
        1
    );
    assert_eq!(
        requests(&u)
            .await
            .iter()
            .filter(|r| r.url.path().ends_with("upload_all"))
            .count(),
        2
    );
    cleanup(u).await;
}

#[tokio::test]
async fn lark_base_write_stale_worker_after_flag_off_sweep_sends_nothing_and_upload_before_persist_reuploads_on_takeover()
 {
    use crate::models::lark_base_write::{COLLECTION_NAME as WRITES, WriteState};
    for takeover in [false, true] {
        let u = setup("base_stale_worker").await;
        let auth = authenticate(&u.f).await;
        Mock::given(method("POST"))
            .and(path("/open-apis/drive/v1/medias/upload_all"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_millis(500))
                    .set_body_json(json!({"code":0,"data":{"file_token":"uploaded"}})),
            )
            .with_priority(1)
            .mount(&u.server)
            .await;
        let (first, second) = tokio::join!(call(&u, &auth, false, args(&u.attachment_id)), async {
            wait_upload(&u).await;
            let row = ledger(&u).await;
            assert!(row.files.is_empty());
            u.f.state.db.collection::<bson::Document>(WRITES).update_one(doc! {"_id":&row.id},doc! {"$set":{"deadline_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
            if takeover {
                Some(call(&u, &auth, true, args(&u.attachment_id)).await)
            } else {
                writes(&u.f, false).await;
                assert_eq!(
                    crate::services::lark_base_writes::sweep(&u.f.state.db)
                        .await
                        .unwrap(),
                    1
                );
                None
            }
        });
        assert_error(&first);
        if let Some(second) = second {
            assert_eq!(payload(&second)["record_id"], "rec_receipt");
            assert_eq!(
                requests(&u)
                    .await
                    .iter()
                    .filter(|r| r.url.path().ends_with("upload_all"))
                    .count(),
                2
            );
            assert_eq!(
                requests(&u)
                    .await
                    .iter()
                    .filter(|r| r.url.path().ends_with("/records"))
                    .count(),
                1
            );
        } else {
            assert_no_create(&u).await;
            assert_eq!(requests(&u).await.len(), 2);
            assert_eq!(ledger(&u).await.state, WriteState::FailedPreDispatch);
        }
        cleanup(u).await;
    }
}

#[tokio::test]
async fn lark_base_write_database_failure_before_barrier_blocks_create_and_after_send_keeps_uncertain_barrier()
 {
    use crate::models::lark_base_write::{COLLECTION_NAME as WRITES, WriteState};
    for after_send in [false, true] {
        let u = setup("base_db_failure").await;
        let auth = authenticate(&u.f).await;
        if after_send {
            Mock::given(method("POST"))
                .and(path("/open-apis/bitable/v1/apps/base/tables/table/records"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_delay(std::time::Duration::from_millis(500))
                        .set_body_json(
                            json!({"code":0,"data":{"record":{"record_id":"rec_receipt"}}}),
                        ),
                )
                .with_priority(1)
                .mount(&u.server)
                .await;
        } else {
            // Force the barrier write to fail, while allowing its pre-dispatch cleanup.
            u.f.state.db.run_command(doc! {"collMod":WRITES,"validator":{"state":{"$ne":"dispatching"}},"validationLevel":"strict"}).await.unwrap();
        }
        let (response, ()) = tokio::join!(call(&u, &auth, false, args(&u.attachment_id)), async {
            if after_send {
                wait_path(&u, "/records").await;
                u.f.state.db.run_command(doc! {"collMod":WRITES,"validator":{"state":"dispatching"},"validationLevel":"strict"}).await.unwrap();
            }
        });
        assert_error(&response);
        if after_send {
            assert_eq!(ledger(&u).await.state, WriteState::Dispatching);
            u.f.state
                .db
                .run_command(doc! {"collMod":WRITES,"validator":{}})
                .await
                .unwrap();
            u.f.state.db.collection::<bson::Document>(WRITES).update_one(doc! {},doc! {"$set":{"deadline_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
            crate::services::lark_base_writes::sweep(&u.f.state.db)
                .await
                .unwrap();
            assert_eq!(ledger(&u).await.state, WriteState::Uncertain);
            assert_error(&call(&u, &auth, true, args(&u.attachment_id)).await);
            assert_eq!(
                requests(&u)
                    .await
                    .iter()
                    .filter(|r| r.url.path().ends_with("/records"))
                    .count(),
                1
            );
        } else {
            assert_no_create(&u).await;
            assert_eq!(ledger(&u).await.state, WriteState::FailedPreDispatch);
        }
        cleanup(u).await;
    }
}

#[tokio::test]
async fn lark_base_write_single_card_binds_canonical_body_and_all_four_contracts_before_any_child_send()
 {
    use crate::services::assistant_acknowledgement_service as acks;
    for dependency in [
        attachment::COMPOSITE_OPERATION,
        attachment::FIELDS_OPERATION,
        attachment::UPLOAD_OPERATION,
        attachment::CREATE_OPERATION,
    ] {
        let u = setup("base_card_contracts").await;
        crate::services::assistant_settings_service::update(
            &u.f.state.db,
            &u.f.owner,
            crate::services::assistant_settings_service::Update {
                access_mode: Some(crate::models::assistant_settings::AccessMode::AskEveryTime),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let auth = authenticate(&u.f).await;
        let response = call(&u, &auth, false, args(&u.attachment_id)).await;
        assert_error(&response);
        let card = payload(&response);
        assert_eq!(card["error"], "acknowledgement_required");
        let id = card["acknowledgement_id"].as_str().unwrap();
        let history = acks::history(&u.f.state.db, &u.f.owner, &u.f.row.id)
            .await
            .unwrap();
        assert_eq!(history.len(), 1);
        // JSON object order must not create a second decision.
        let reordered = json!({"attachments":{"Receipt":[u.attachment_id]},"fields":{"Date":1700000000000i64,"Card Number":"1234","Type":"Meal","Amount":12.34},"table_id":"table","app_token":"base"});
        assert_eq!(
            payload(&call(&u, &auth, true, reordered).await)["acknowledgement_id"],
            id
        );
        acks::decide(&u.f.state.db, &u.f.owner, &u.f.row.id, id, true)
            .await
            .unwrap();
        let mut approved = args(&u.attachment_id);
        approved["acknowledgement_id"] = json!(id);
        let mut changed = approved.clone();
        changed["fields"]["Amount"] = json!(99.99);
        assert_error(&call(&u, &auth, false, changed).await);
        assert!(requests(&u).await.is_empty());
        u.f.state
            .db
            .collection::<bson::Document>(service_endpoint::COLLECTION_NAME)
            .update_one(
                doc! {"service_id":&u.catalog.id,"name":dependency},
                doc! {"$set":{"path":"/edited-after-approval"}},
            )
            .await
            .unwrap();
        assert_error(&call(&u, &auth, true, approved).await);
        assert!(requests(&u).await.is_empty());
        cleanup(u).await;
    }
    let u = setup("base_card_expansion").await;
    crate::services::assistant_settings_service::update(
        &u.f.state.db,
        &u.f.owner,
        crate::services::assistant_settings_service::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::AskEveryTime),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let auth = authenticate(&u.f).await;
    let card = payload(&call(&u, &auth, false, args(&u.attachment_id)).await);
    let id = card["acknowledgement_id"].as_str().unwrap();
    acks::decide(&u.f.state.db, &u.f.owner, &u.f.row.id, id, true)
        .await
        .unwrap();
    let mut approved = args(&u.attachment_id);
    approved["acknowledgement_id"] = json!(id);
    assert_eq!(
        payload(&call(&u, &auth, true, approved).await)["record_id"],
        "rec_receipt"
    );
    assert_eq!(requests(&u).await.len(), 3);
    assert_eq!(
        acks::history(&u.f.state.db, &u.f.owner, &u.f.row.id)
            .await
            .unwrap()
            .len(),
        1
    );
    cleanup(u).await;
}

#[tokio::test]
async fn lark_base_write_final_create_revalidates_after_delayed_tenant_token_preparation() {
    for drift in ["key", "stop", "credential", "contract", "token_preparation"] {
        let u = setup("base_final_send").await;
        let exchange = PausedTokenExchange::start(drift == "token_preparation").await;
        let credential = use_paused_tenant_exchange(&u, &exchange).await;
        let auth = authenticate(&u.f).await;
        let (response, ()) = tokio::join!(call(&u, &auth, false, args(&u.attachment_id)), async {
            tokio::time::timeout(
                std::time::Duration::from_secs(20),
                exchange.reached.notified(),
            )
            .await
            .unwrap();
            assert_eq!(
                ledger(&u).await.state,
                crate::models::lark_base_write::WriteState::Dispatching
            );
            match drift {
                "key" => {
                    u.f.state
                        .db
                        .collection::<bson::Document>("api_keys")
                        .update_one(
                            doc! {"_id":&u.f.chat.api_key_id},
                            doc! {"$set":{"is_active":false}},
                        )
                        .await
                        .unwrap();
                }
                "stop" => {
                    u.f.state
                        .db
                        .collection::<bson::Document>("assistant_conversations")
                        .update_one(
                            doc! {"_id":&u.f.row.id},
                            doc! {"$set":{"active_turn.stop_requested":true}},
                        )
                        .await
                        .unwrap();
                }
                "credential" => {
                    bump_credential_epoch(&u.f, &credential, ledger(&u).await.principal.epoch)
                        .await;
                }
                "token_preparation" => {}
                _ => {
                    u.f.state
                        .db
                        .collection::<bson::Document>(service_endpoint::COLLECTION_NAME)
                        .update_one(
                            doc! {"service_id":&u.catalog.id,"name":attachment::CREATE_OPERATION},
                            doc! {"$set":{"publication":"draft"}},
                        )
                        .await
                        .unwrap();
                }
            }
            exchange.release.notify_one();
        });
        assert_eq!(response["result"]["isError"], true, "{drift}: {response}");
        assert_no_create(&u).await;
        let failed = ledger(&u).await;
        assert_eq!(
            failed.state,
            crate::models::lark_base_write::WriteState::FailedDefinite,
            "{drift}"
        );
        assert_eq!(failed.last_code.as_deref(), Some("not_sent"), "{drift}");
        assert_eq!(
            u.f.state
                .db
                .collection::<bson::Document>(crate::models::lark_base_write_photo::COLLECTION_NAME)
                .count_documents(doc! {"write_id":&failed.id})
                .await
                .unwrap(),
            0,
            "{drift}"
        );
        match drift {
            "key" => {
                u.f.state
                    .db
                    .collection::<bson::Document>("api_keys")
                    .update_one(
                        doc! {"_id":&u.f.chat.api_key_id},
                        doc! {"$set":{"is_active":true}},
                    )
                    .await
                    .unwrap();
            }
            "stop" => {
                u.f.state
                    .db
                    .collection::<bson::Document>("assistant_conversations")
                    .update_one(
                        doc! {"_id":&u.f.row.id},
                        doc! {"$set":{"active_turn.stop_requested":false}},
                    )
                    .await
                    .unwrap();
            }
            "contract" => {
                u.f.state
                    .db
                    .collection::<bson::Document>(service_endpoint::COLLECTION_NAME)
                    .update_one(
                        doc! {"service_id":&u.catalog.id,"name":attachment::CREATE_OPERATION},
                        doc! {"$set":{"publication":"published"}},
                    )
                    .await
                    .unwrap();
            }
            _ => {}
        }
        let fresh_auth = authenticate(&u.f).await;
        assert_eq!(
            payload(&call(&u, &fresh_auth, true, args(&u.attachment_id)).await)["record_id"],
            "rec_receipt",
            "{drift}"
        );
        let succeeded = ledger(&u).await;
        assert_ne!(succeeded.client_token, failed.client_token, "{drift}");
        assert_eq!(
            succeeded.state,
            crate::models::lark_base_write::WriteState::Succeeded,
            "{drift}"
        );
        let sent = requests(&u).await;
        let creates: Vec<_> = sent
            .iter()
            .filter(|request| request.url.path().ends_with("/records"))
            .collect();
        assert_eq!(creates.len(), 1, "{drift}");
        assert_eq!(
            creates[0]
                .url
                .query_pairs()
                .find(|(key, _)| key == "client_token")
                .unwrap()
                .1,
            succeeded.client_token
        );
        assert_eq!(
            u.f.state
                .db
                .collection::<bson::Document>(crate::models::lark_base_write_photo::COLLECTION_NAME)
                .count_documents(doc! {"write_id":&succeeded.id,"attempt":&succeeded.attempt})
                .await
                .unwrap(),
            1,
            "{drift}"
        );
        cleanup(u).await;
    }
}

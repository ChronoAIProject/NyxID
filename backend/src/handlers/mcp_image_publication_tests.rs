//! The real owner-chat MCP path, two pinned TLS recipients and a real replica set.
use super::*;
use crate::handlers::credential_record_fixtures::KeySecret;
use crate::models::{
    downstream_service::{self, DownstreamService},
    service_endpoint::{self, PublicationState, ServiceEndpoint},
};
use crate::services::{
    assistant_acknowledgement_service as acks,
    assistant_authority_tests::{Fixture, connected, orchestrator_fixture},
    assistant_upload_service as uploads, catalog_spec_registry, catalog_spec_sync,
    image_publication as publication, service_endpoint_service,
};
use chrono::Utc;
use mongodb::bson::{self, doc};
use nyxid_service_adapters::{
    image_publication::{FixturePublisher, ImagePublisher, LinkedIn},
    test_support::{self, ScriptedFixture},
};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

struct Test {
    f: Fixture,
    connection: String,
    catalog: DownstreamService,
    attachment: String,
    bytes: Vec<u8>,
    api: ScriptedFixture,
    upload: ScriptedFixture,
    company: String,
    caption: String,
    input: Value,
    builder: Arc<dyn Fn() -> reqwest::ClientBuilder + Send + Sync>,
}
fn response(status: u16, headers: &[(&str, &str)], body: Value) -> String {
    let body = if body.is_null() {
        String::new()
    } else {
        body.to_string()
    };
    format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n{body}",
        body.len(),
        headers
            .iter()
            .map(|(k, v)| format!("{k}: {v}\r\n"))
            .collect::<String>()
    )
}
async fn setup(name: &str, fixture: bool, create_status: u16, receipt_id: Option<&str>) -> Test {
    setup_scenario(name, fixture, create_status, receipt_id, "happy").await
}
async fn setup_scenario(
    name: &str,
    fixture: bool,
    create_status: u16,
    receipt_id: Option<&str>,
    scenario: &str,
) -> Test {
    let f = orchestrator_fixture(name).await;
    crate::db::ensure_image_publication_indexes(&f.state.db)
        .await
        .unwrap();
    crate::services::assistant_settings_service::update(
        &f.state.db,
        &f.owner,
        crate::services::assistant_settings_service::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::Autopilot),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let (api_host, upload_host, company, image, post, upload_path) = if fixture {
        (
            "fixture.publisher.test",
            "fixture.upload.test",
            "page:company",
            "file-1",
            "entry-1",
            "/bytes",
        )
    } else {
        (
            "api.linkedin.com",
            "www.linkedin.com",
            "urn:li:organization:18482858",
            "urn:li:image:abc123",
            "urn:li:share:987654321",
            "/dms-uploads/original",
        )
    };
    let adapter: &dyn ImagePublisher = if fixture {
        &FixturePublisher
    } else {
        &LinkedIn
    };
    let caption = "Rona's [image] | @news #launch *literal* \\ 😀".to_owned();
    let until = (Utc::now()
        + chrono::Duration::minutes(if scenario == "expired" { -10 } else { 10 }))
    .timestamp_millis();
    let upload_url = format!("https://{upload_host}{upload_path}?signature=exact%2Fvalue&x=1");
    let upload_url = if scenario == "recipient" {
        "https://www.linkedin.com.evil/dms-uploads/file".to_owned()
    } else {
        upload_url
    };
    let init = if fixture {
        json!({"put":upload_url,"file":image,"until":until})
    } else {
        json!({"value":{"uploadUrl":upload_url,"image":image,"uploadUrlExpiresAt":until}})
    };
    let mut ready = if fixture {
        json!({"account":company,"file":image,"ready":true})
    } else {
        json!({"id":image,"owner":company,"status":"AVAILABLE"})
    };
    if scenario == "wrong_owner" {
        ready["owner"] = json!("urn:li:organization:999");
    }
    if scenario == "processing_failed" {
        ready["status"] = json!("PROCESSING_FAILED");
    }
    let mut read = adapter.post(company, &caption, image).body.unwrap();
    read["id"] = json!(if scenario == "wrong_post" {
        "urn:li:share:111"
    } else {
        post
    });
    let header_name = if fixture { "fixture-id" } else { "x-restli-id" };
    let headers = receipt_id
        .map(|p| vec![(header_name, p)])
        .unwrap_or_default();
    let create = if scenario == "lost_create" {
        String::new()
    } else if scenario == "body_stall" {
        format!(
            "HTTP/1.1 201 Created\r\nContent-Length: 999999\r\nx-restli-id: {}\r\nConnection: close\r\n\r\n",
            receipt_id.unwrap()
        )
    } else {
        response(create_status, &headers, Value::Null)
    };
    let mut responses = vec![response(200, &[], init)];
    if scenario == "pending" {
        let mut pending = ready.clone();
        pending["status"] = json!("PROCESSING");
        responses.extend((0..12).map(|_| response(200, &[], pending.clone())));
    }
    if scenario == "slow_processing" {
        let mut pending = ready.clone();
        pending["status"] = json!("PROCESSING");
        responses.extend([
            response(200, &[], pending.clone()),
            response(200, &[], pending),
        ]);
    }
    if scenario == "revisions" {
        responses.extend((0..8).map(|_| response(200, &[], ready.clone())));
    }
    responses.extend([
        response(200, &[], ready.clone()),
        response(200, &[], ready),
        create,
        response(200, &[], read.clone()),
        response(200, &[], read),
    ]);
    if scenario == "independent_media" {
        responses = vec![
            response(201, &[], json!({"id":"urn:li:share:111"})),
            response(201, &[], json!({"id":"urn:li:share:222"})),
        ];
    }
    let api = test_support::scripted_fixture(api_host, responses).await;
    let upload_response = if scenario == "redirect" {
        response(307, &[("Location", "https://evil.test/bytes")], Value::Null)
    } else {
        response(201, &[], Value::Null)
    };
    let upload = test_support::scripted_fixture(upload_host, vec![upload_response]).await;
    let api_addr = api.address;
    let upload_addr = upload.address;
    let cert_api = api.certificate.clone();
    let cert_upload = upload.certificate.clone();
    let builder: Arc<dyn Fn() -> reqwest::ClientBuilder + Send + Sync> = Arc::new(move || {
        reqwest::Client::builder()
            .no_proxy()
            .resolve(api_host, api_addr)
            .resolve(upload_host, upload_addr)
            .add_root_certificate(cert_api.clone())
            .add_root_certificate(cert_upload.clone())
    });
    let mut catalog = downstream_service::test_helpers::dummy_service();
    catalog.id = Uuid::new_v4().to_string();
    catalog.name = "Publishing".into();
    catalog.slug = if fixture {
        "fixture-publisher"
    } else {
        "api-linkedin"
    }
    .into();
    catalog.created_by = "system".into();
    catalog.base_url = adapter.api_origin().into();
    f.state
        .db
        .collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
        .insert_one(&catalog)
        .await
        .unwrap();
    let connection = connected(&f.state.db, &f.owner, "publisher", adapter.api_origin()).await;
    let credential = Uuid::new_v4().to_string();
    KeySecret::AccessToken.insert(&f.state.db,&f.state.encryption_keys,doc! {"_id":&credential,"user_id":&f.owner,"label":"Rona BYO","credential_type":"oauth2","status":"active","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()},b"selected-byo-oauth").await;
    f.state.db.collection::<bson::Document>("user_services").update_one(doc! {"_id":&connection},doc! {"$set":{"catalog_service_id":&catalog.id,"api_key_id":credential,"auth_method":"bearer"}}).await.unwrap();
    let spec = catalog_spec_registry::spec_for_key(
        publication::connector(&catalog).unwrap().managed_spec_key,
    )
    .unwrap();
    for input in catalog_spec_sync::destination_endpoint_inputs(&catalog, &spec).unwrap() {
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
    }
    let mut bytes = std::io::Cursor::new(Vec::new());
    let image = image::DynamicImage::new_rgb8(2, 2);
    image
        .write_to(
            &mut bytes,
            if fixture || scenario == "jpeg" {
                image::ImageFormat::Jpeg
            } else {
                image::ImageFormat::Png
            },
        )
        .unwrap();
    let bytes = bytes.into_inner();
    let attachment = bind_file(
        &f,
        if fixture || scenario == "jpeg" {
            "original.jpg"
        } else {
            "original.png"
        },
        &bytes,
    )
    .await;
    let input = json!({"action":"preview","request_id":Uuid::new_v4().to_string(),"company":company,"caption":caption,"attachment_id":attachment});
    Test {
        f,
        connection,
        catalog,
        attachment,
        bytes,
        api,
        upload,
        company: company.into(),
        caption,
        input,
        builder,
    }
}
async fn bind_file(f: &Fixture, name: &str, bytes: &[u8]) -> String {
    let item = uploads::upload(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
        name,
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
async fn auth(f: &Fixture) -> McpAuthContext {
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
async fn call(t: &Test, arguments: Value, meta: bool) -> Value {
    call_operation(t, publication::OPERATION, arguments, meta).await
}
async fn call_operation(t: &Test, operation: &str, arguments: Value, meta: bool) -> Value {
    crate::services::proxy_service::TARGET_HTTP_CLIENT_BUILDER.scope(t.builder.clone(),async {
        let auth=auth(&t.f).await;
        let name=format!("publisher__{operation}");
        let params=if meta {json!({"name":"nyx__call_tool","arguments":{"tool_name":name,"arguments_json":arguments.to_string()}})} else {json!({"name":name,"arguments":arguments})};
        let request=JsonRpcRequest {jsonrpc:"2.0".into(),id:Some(json!(1)),method:"tools/call".into(),params:Some(params)};
        let response=Box::pin(handle_tools_call(&t.f.state,&auth,None,&request,false,crate::services::billing::route_inventory::internal_node_dispatch_permit())).await;
        let body:Value=serde_json::from_slice(&axum::body::to_bytes(response.into_body(),2*1024*1024).await.unwrap()).unwrap();
        let text=body["result"]["content"][0]["text"].as_str().unwrap_or_else(||panic!("{body}"));
        let payload = if text.starts_with("Error (") {
            text.split_once("): ").map(|(_,payload)|payload).unwrap_or(text)
        } else {text};
        let mut value=serde_json::from_str::<Value>(payload).unwrap_or_else(|_|json!({"raw":text,"rpc":body}));
        value["_is_error"]=body["result"]["isError"].clone();
        value
    }).await
}
fn publish_input(t: &Test, draft: &Value) -> Value {
    json!({"action":"publish","draft_id":draft["draft_id"],"company":t.company,"caption":t.caption,"attachment_id":t.attachment})
}
async fn approve(t: &Test, card: &Value) -> String {
    assert_eq!(card["error"], "acknowledgement_required", "{card}");
    let id = card["acknowledgement_id"].as_str().unwrap();
    acks::decide(&t.f.state.db, &t.f.owner, &t.f.row.id, id, true)
        .await
        .unwrap();
    id.into()
}

#[tokio::test]
async fn image_publication_two_adapters_prepare_transfer_ready_human_receipt_and_replay() {
    for fixture in [false, true] {
        let post = if fixture {
            "entry-1"
        } else {
            "urn:li:share:987654321"
        };
        let mut t = setup("image2949_two_adapters", fixture, 201, Some(post)).await;
        let preview = call(&t, t.input.clone(), fixture).await;
        assert_eq!(preview["state"], "ready", "{preview}");
        let retry = call(&t, t.input.clone(), fixture).await;
        assert_eq!(retry["draft_id"], preview["draft_id"], "{retry}");
        assert_eq!(retry["preview"], preview["preview"], "{retry}");
        let private_image = if fixture {
            "file-1"
        } else {
            "urn:li:image:abc123"
        };
        assert!(
            !preview.to_string().contains(private_image),
            "prepared asset leaked: {preview}"
        );
        let pending = call(
            &t,
            json!({"action":"status","draft_id":preview["draft_id"]}),
            fixture,
        )
        .await;
        assert!(
            !pending.to_string().contains(private_image),
            "prepared asset leaked: {pending}"
        );
        assert_eq!(preview["preview"]["caption"], t.caption);
        assert!(
            preview["preview"]["image_endpoint"]
                .as_str()
                .unwrap()
                .contains(&t.attachment)
        );
        let sent =
            tokio::time::timeout(std::time::Duration::from_secs(1), t.upload.requests.recv())
                .await
                .unwrap()
                .unwrap();
        let end = sent.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        assert_eq!(&sent[end..], &t.bytes);
        assert!(t.upload.requests.try_recv().is_err());
        let head = String::from_utf8_lossy(&sent[..end]).to_lowercase();
        assert!(head.contains("signature=exact%2fvalue&x=1"));
        assert_eq!(
            head.contains("authorization: bearer selected-byo-oauth"),
            !fixture
        );
        assert!(!head.contains("x-nyxid"));
        if fixture {
            assert!(head.contains("fixture-upload: original"));
        }
        let mut input = publish_input(&t, &preview);
        let card = call(&t, input.clone(), fixture).await;
        assert!(
            !card.to_string().contains(private_image),
            "prepared asset leaked: {card}"
        );
        let id = approve(&t, &card).await;
        let stored =
            t.f.state
                .db
                .collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(
                    "assistant_acknowledgements",
                )
                .find_one(doc! {"_id":&id})
                .await
                .unwrap()
                .unwrap();
        let review = stored.confirmation.unwrap().image_publication.unwrap();
        assert_eq!(review.caption, t.caption);
        assert_eq!(review.company, t.company);
        assert_eq!(review.attachment_id, t.attachment);
        input["acknowledgement_id"] = json!(id);
        let result = call(&t, input.clone(), fixture).await;
        assert_eq!(result["state"], "created", "{result}");
        assert_eq!(result["verification"], "verified", "{result}");
        assert_eq!(result["post_urn"], post);
        assert_eq!(result["image"], private_image);
        let replay = call(&t, input, fixture).await;
        assert_eq!(replay["post_urn"], post, "{replay}");
        let status = call(
            &t,
            json!({"action":"status","draft_id":preview["draft_id"]}),
            fixture,
        )
        .await;
        assert_eq!(status["verification"], "verified", "{status}");
        // One init, two readiness reads, exactly one create, two readbacks.
        let mut requests = Vec::new();
        while let Ok(request) = t.api.requests.try_recv() {
            requests.push(String::from_utf8_lossy(&request).to_string());
        }
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.starts_with(if fixture {
                    "POST /entries "
                } else {
                    "POST /rest/posts "
                }))
                .count(),
            1,
            "{requests:?}"
        );
        t.f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn image_publication_negative_provider_paths_stop_before_post() {
    for (scenario, code, stage) in [
        ("recipient", "upload_recipient_refused", "initialization"),
        ("expired", "upload_recipient_refused", "upload"),
        ("wrong_owner", "image_owner_mismatch", "readiness"),
        ("processing_failed", "image_processing_failed", "readiness"),
        ("redirect", "provider_or_authority_refused", "upload"),
    ] {
        let mut t = setup_scenario(
            "image2949_negative",
            false,
            201,
            Some("urn:li:share:987654321"),
            scenario,
        )
        .await;
        let preview = call(&t, t.input.clone(), false).await;
        assert_eq!(preview["state"], "upload_failed", "{scenario}: {preview}");
        assert_eq!(preview["_is_error"], true, "{scenario}: {preview}");
        assert_eq!(preview["code"], code, "{scenario}: {preview}");
        assert_eq!(preview["stage"], stage, "{scenario}: {preview}");
        let publish = call(&t, publish_input(&t, &preview), false).await;
        assert!(
            publish["raw"]
                .as_str()
                .unwrap_or_default()
                .contains("draft_not_ready"),
            "{publish}"
        );
        let mut requests = Vec::new();
        while let Ok(request) = t.api.requests.try_recv() {
            requests.push(String::from_utf8_lossy(&request).to_string());
        }
        assert!(
            !requests.iter().any(|r| r.starts_with("POST /rest/posts ")),
            "{requests:?}"
        );
        t.f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn image_publication_receipt_failures_never_resubmit_or_derive_error_links() {
    for (scenario, status, id, state, verification) in [
        ("happy", 201, None, "created", "receipt_missing"),
        ("happy", 201, Some("bad-id"), "created", "receipt_missing"),
        (
            "happy",
            403,
            Some("urn:li:share:987654321"),
            "failed",
            "pending_readback",
        ),
        ("lost_create", 201, None, "unknown", "outcome_unknown"),
        (
            "body_stall",
            201,
            Some("urn:li:share:987654321"),
            "created",
            "verified",
        ),
        (
            "wrong_post",
            201,
            Some("urn:li:share:987654321"),
            "created",
            "mismatch",
        ),
    ] {
        let mut t = setup_scenario("image2949_receipts", false, status, id, scenario).await;
        let preview = call(&t, t.input.clone(), false).await;
        assert_eq!(preview["state"], "ready", "{preview}");
        let mut input = publish_input(&t, &preview);
        let card = call(&t, input.clone(), false).await;
        input["acknowledgement_id"] = json!(approve(&t, &card).await);
        let result = call(&t, input.clone(), false).await;
        assert_eq!(result["state"], state, "{scenario}: {result}");
        assert_eq!(
            result["_is_error"],
            state != "created",
            "{scenario}: {result}"
        );
        assert_eq!(result["verification"], verification, "{scenario}: {result}");
        if state != "created" {
            assert!(result["post_url"].is_null(), "{result}");
            assert!(result["image"].is_null(), "{result}");
        }
        let replay = call(&t, input, false).await;
        assert_eq!(replay["state"], state, "{replay}");
        let mut requests = Vec::new();
        while let Ok(request) = t.api.requests.try_recv() {
            requests.push(String::from_utf8_lossy(&request).to_string());
        }
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.starts_with("POST /rest/posts "))
                .count(),
            1,
            "{requests:?}"
        );
        t.f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn image_publication_revisions_cancel_edit_back_retries_and_exact_identity() {
    let t = setup_scenario(
        "image2949_revisions",
        false,
        201,
        Some("urn:li:share:987654321"),
        "revisions",
    )
    .await;
    let a = call(&t, t.input.clone(), false).await;
    assert_eq!(a["state"], "ready", "{a}");
    let card = call(&t, publish_input(&t, &a), false).await;
    let mut b_input = t.input.clone();
    b_input["request_id"] = json!(Uuid::new_v4().to_string());
    b_input["draft_id"] = a["draft_id"].clone();
    b_input["caption"] = json!("Edited caption");
    let b = call(&t, b_input.clone(), false).await;
    assert_eq!(b["state"], "ready", "{b}");
    let old = call(&t, publish_input(&t, &a), false).await;
    assert!(
        old["raw"]
            .as_str()
            .unwrap_or_default()
            .contains("draft_not_ready"),
        "{old}"
    );
    let saved =
        t.f.state
            .db
            .collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(
                "assistant_acknowledgements",
            )
            .find_one(doc! {"_id":card["acknowledgement_id"].as_str().unwrap()})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(saved.status, "expired");
    let mut back = t.input.clone();
    back["request_id"] = json!(Uuid::new_v4().to_string());
    back["draft_id"] = b["draft_id"].clone();
    // Reuse the already uploaded image; each caption revision still rechecks readiness.
    let again = call(&t, back.clone(), false).await;
    assert_ne!(again["draft_id"], a["draft_id"], "{again}");
    assert_eq!(again["state"], "ready", "{again}");
    let cancelled = call(
        &t,
        json!({"action":"cancel","draft_id":again["draft_id"]}),
        false,
    )
    .await;
    assert_eq!(cancelled["state"], "cancelled", "{cancelled}");
    let retry = call(&t, back.clone(), false).await;
    assert_eq!(retry["draft_id"], again["draft_id"]);
    assert_eq!(retry["state"], "cancelled");
    let mut deliberate = t.input.clone();
    deliberate["request_id"] = json!(Uuid::new_v4().to_string());
    deliberate["draft_id"] = again["draft_id"].clone();
    let fresh = call(&t, deliberate, false).await;
    assert_eq!(fresh["state"], "ready", "{fresh}");
    assert_ne!(fresh["draft_id"], again["draft_id"]);
    let mut altered = back;
    altered["caption"] = json!("Must use a new request ID");
    let refusal = call(&t, altered, false).await;
    assert!(
        refusal["raw"]
            .as_str()
            .unwrap_or_default()
            .contains("request_id_payload_changed"),
        "{refusal}"
    );
    t.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn image_publication_status_uses_digest_without_retained_file_and_jpeg_slow_readiness() {
    let t = setup_scenario(
        "image2949_slow",
        false,
        201,
        Some("urn:li:share:987654321"),
        "slow_processing",
    )
    .await;
    let preview = call(&t, t.input.clone(), false).await;
    assert_eq!(preview["state"], "ready", "{preview}");
    let mut input = publish_input(&t, &preview);
    let card = call(&t, input.clone(), false).await;
    input["acknowledgement_id"] = json!(approve(&t, &card).await);
    assert_eq!(call(&t, input, false).await["verification"], "verified");
    t.f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
        .delete_many(doc! {"$or":[{"_id":&t.attachment},{"parent_attachment_id":&t.attachment}]})
        .await
        .unwrap();
    let status = call(
        &t,
        json!({"action":"status","draft_id":preview["draft_id"]}),
        false,
    )
    .await;
    assert_eq!(status["verification"], "verified", "{status}");
    t.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn image_publication_preparation_and_publication_have_separate_owner_decisions() {
    for mode in [
        crate::models::assistant_settings::AccessMode::Nyxmode,
        crate::models::assistant_settings::AccessMode::AskEveryTime,
    ] {
        let mut t = setup(
            "image2949_two_decisions",
            false,
            201,
            Some("urn:li:share:987654321"),
        )
        .await;
        crate::services::assistant_settings_service::update(
            &t.f.state.db,
            &t.f.owner,
            crate::services::assistant_settings_service::Update {
                access_mode: Some(mode),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let upload_card = call(&t, t.input.clone(), false).await;
        assert_eq!(
            upload_card["error"], "acknowledgement_required",
            "{upload_card}"
        );
        assert!(t.api.requests.try_recv().is_err());
        assert!(t.upload.requests.try_recv().is_err());
        let mut prepare = t.input.clone();
        prepare["acknowledgement_id"] = json!(approve(&t, &upload_card).await);
        let preview = call(&t, prepare.clone(), false).await;
        assert_eq!(preview["state"], "ready", "{preview}");
        let repeated = call(&t, prepare, false).await;
        assert_eq!(repeated["draft_id"], preview["draft_id"], "{repeated}");
        assert_eq!(repeated["state"], "ready", "{repeated}");
        let publish = call(&t, publish_input(&t, &preview), false).await;
        assert_eq!(publish["error"], "acknowledgement_required", "{publish}");
        assert_ne!(
            publish["acknowledgement_id"],
            upload_card["acknowledgement_id"]
        );
        acks::decide(
            &t.f.state.db,
            &t.f.owner,
            &t.f.row.id,
            publish["acknowledgement_id"].as_str().unwrap(),
            false,
        )
        .await
        .unwrap();
        let mut denied = publish_input(&t, &preview);
        denied["acknowledgement_id"] = publish["acknowledgement_id"].clone();
        let refusal = call(&t, denied, false).await;
        assert_ne!(refusal["state"], "created", "{refusal}");
        let mut requests = Vec::new();
        while let Ok(request) = t.api.requests.try_recv() {
            requests.push(String::from_utf8_lossy(&request).to_string());
        }
        assert!(!requests.iter().any(|r| r.starts_with("POST /rest/posts ")));
        t.f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn image_publication_approved_draft_refuses_changed_page_file_or_credential() {
    let mut t = setup(
        "image2949_immutable",
        false,
        201,
        Some("urn:li:share:987654321"),
    )
    .await;
    let preview = call(&t, t.input.clone(), false).await;
    assert_eq!(preview["state"], "ready", "{preview}");
    let mut input = publish_input(&t, &preview);
    let card = call(&t, input.clone(), false).await;
    input["acknowledgement_id"] = json!(approve(&t, &card).await);
    let another = bind_file(&t.f, "other.png", &t.bytes).await;
    for field in ["company", "caption", "attachment_id"] {
        let mut drift = input.clone();
        drift[field] = json!(match field {
            "company" => "urn:li:organization:2",
            "caption" => "Changed caption",
            _ => &another,
        });
        let refusal = call(&t, drift, false).await;
        assert!(
            refusal["raw"]
                .as_str()
                .unwrap_or_default()
                .contains("draft_changed"),
            "{refusal}"
        );
    }
    let instance =
        t.f.state
            .db
            .collection::<bson::Document>("user_services")
            .find_one(doc! {"_id":&t.connection})
            .await
            .unwrap()
            .unwrap();
    let binding_before = crate::services::assistant_service_execution::connection_binding(
        &t.f.state.db,
        &t.connection,
        &t.f.chat.api_key_id,
    )
    .await
    .unwrap();
    let credential_id = instance.get_str("api_key_id").unwrap();
    crate::services::user_api_key_service::update_api_key(
        &t.f.state.db,
        &t.f.state.encryption_keys,
        &t.f.owner,
        credential_id,
        None,
        Some("rotated-byo-oauth"),
    )
    .await
    .unwrap();
    let credential =
        t.f.state
            .db
            .collection::<crate::models::user_api_key::UserApiKey>("user_api_keys")
            .find_one(doc! {"_id":credential_id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(credential.credential_epoch, 2);
    let binding_after = crate::services::assistant_service_execution::connection_binding(
        &t.f.state.db,
        &t.connection,
        &t.f.chat.api_key_id,
    )
    .await
    .unwrap();
    assert_ne!(
        binding_before, binding_after,
        "rotation must change the live authority binding"
    );
    let refusal = call(&t, input, false).await;
    assert!(
        refusal["raw"]
            .as_str()
            .unwrap_or_default()
            .contains("draft_changed"),
        "{refusal}"
    );
    let mut requests = Vec::new();
    while let Ok(request) = t.api.requests.try_recv() {
        requests.push(String::from_utf8_lossy(&request).to_string());
    }
    assert!(!requests.iter().any(|r| r.starts_with("POST /rest/posts ")));
    t.f.state
        .db
        .collection::<DownstreamService>(downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&t.catalog.id},
            doc! {"$set":{"is_active":false}},
        )
        .await
        .unwrap();
    let caller = auth(&t.f).await;
    let services = load_all_services_for_meta_tools(&t.f.state, &caller)
        .await
        .unwrap();
    assert!(
        services
            .iter()
            .flat_map(|s| &s.endpoints)
            .all(|e| !publication::candidate(e))
    );
    t.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn image_publication_pending_readiness_resumes_by_id_without_reupload() {
    let mut t = setup_scenario(
        "image2949_pending",
        false,
        201,
        Some("urn:li:share:987654321"),
        "pending",
    )
    .await;
    let preview = call(&t, t.input.clone(), false).await;
    assert_eq!(preview["state"], "processing", "{preview}");
    assert_eq!(preview["code"], "image_readiness_timeout");
    let early = call(&t, publish_input(&t, &preview), false).await;
    assert!(
        early["raw"]
            .as_str()
            .unwrap_or_default()
            .contains("draft_not_ready"),
        "{early}"
    );
    assert!(t.upload.requests.try_recv().is_ok());
    let resumed = call(
        &t,
        json!({"action":"status","draft_id":preview["draft_id"]}),
        false,
    )
    .await;
    assert_eq!(resumed["state"], "ready", "{resumed}");
    assert!(t.upload.requests.try_recv().is_err());
    t.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn image_publication_mounted_contract_keeps_text_analytics_and_ordinary_callers_get_no_capability()
 {
    use crate::services::api_docs_service::{SpecCacheTestGuard, cache_test_spec};
    let _cache = SpecCacheTestGuard::acquire();
    let t = setup(
        "image2949_mounted",
        false,
        201,
        Some("urn:li:share:987654321"),
    )
    .await;
    let spec_url = format!("https://spec.fixture.test/{}.json", t.f.owner);
    let spec = json!({"openapi":"3.1.0","info":{"title":"Rona custom","version":"1"},"servers":[{"url":"https://api.linkedin.com"}],"paths":{
        "/v2/ugcPosts":{"post":{"operationId":"create_post","responses":{"201":{"description":"created"}}}},
        "/analytics/1":{"get":{"operationId":"analytics_one","responses":{"200":{"description":"stats"}}}},
        "/analytics/2":{"get":{"operationId":"analytics_two","responses":{"200":{"description":"stats"}}}},
        "/analytics/3":{"get":{"operationId":"analytics_three","responses":{"200":{"description":"stats"}}}},
        "/analytics/4":{"get":{"operationId":"analytics_four","responses":{"200":{"description":"stats"}}}},
        "/analytics/5":{"get":{"operationId":"analytics_five","responses":{"200":{"description":"stats"}}}}
    }});
    cache_test_spec(&spec_url, Some(&t.f.owner), spec);
    let connection =
        t.f.state
            .db
            .collection::<bson::Document>("user_services")
            .find_one(doc! {"_id":&t.connection})
            .await
            .unwrap()
            .unwrap();
    t.f.state
        .db
        .collection::<bson::Document>("user_endpoints")
        .update_one(
            doc! {"_id":connection.get_str("endpoint_id").unwrap()},
            doc! {"$set":{"openapi_spec_url":&spec_url}},
        )
        .await
        .unwrap();
    let caller = auth(&t.f).await;
    let mut services = load_all_services_for_meta_tools(&t.f.state, &caller)
        .await
        .unwrap();
    let service = services
        .iter()
        .find(|s| s.service_id == t.connection)
        .unwrap();
    let names: std::collections::HashSet<_> =
        service.endpoints.iter().map(|e| e.name.as_str()).collect();
    for name in [
        "create_post",
        "analytics_one",
        "analytics_two",
        "analytics_three",
        "analytics_four",
        "analytics_five",
        publication::OPERATION,
    ] {
        assert!(names.contains(name), "{names:?}");
    }
    publication::filter_discovery(&t.f.state.db, None, &mut services)
        .await
        .unwrap();
    let ordinary = services
        .iter()
        .find(|s| s.service_id == t.connection)
        .unwrap();
    assert_eq!(ordinary.endpoints.len(), 6);
    assert!(
        ordinary
            .endpoints
            .iter()
            .all(|e| !publication::candidate(e))
    );
    let preview = call(&t, t.input.clone(), false).await;
    assert_eq!(preview["state"], "ready", "{preview}");
    crate::services::assistant_nyxagent::delete(&t.f.state.db, &t.f.owner, &t.f.row.id)
        .await
        .unwrap();
    assert_eq!(
        t.f.state
            .db
            .collection::<bson::Document>(crate::models::image_publication::COLLECTION_NAME)
            .count_documents(doc! {"user_id":&t.f.owner})
            .await
            .unwrap(),
        0
    );
    t.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn image_publication_linkedin_jpeg_transfers_original_image() {
    let mut t = setup_scenario(
        "image2949_jpeg",
        false,
        201,
        Some("urn:li:share:987654321"),
        "jpeg",
    )
    .await;
    let preview = call(&t, t.input.clone(), false).await;
    assert_eq!(preview["state"], "ready", "{preview}");
    assert_eq!(preview["preview"]["content_type"], "image/jpeg");
    let sent = t.upload.requests.recv().await.unwrap();
    let end = sent.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    assert_eq!(&sent[end..], &t.bytes);
    assert!(
        String::from_utf8_lossy(&sent[..end])
            .to_lowercase()
            .contains("content-type: image/jpeg")
    );
    t.f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn image_publication_preserves_independent_provider_media_and_text_contracts() {
    let mut t = setup_scenario(
        "image2949_independent",
        false,
        201,
        None,
        "independent_media",
    )
    .await;
    for category in ["IMAGE", "NONE"] {
        let mut content = json!({"shareCommentary":{"text":"Independent existing content"},"shareMediaCategory":category});
        if category == "IMAGE" {
            content["media"] =
                json!([{"status":"READY","media":"urn:li:digitalmediaAsset:independent"}]);
        }
        let arguments = json!({"author":t.company,"lifecycleState":"PUBLISHED",
            "specificContent":{"com.linkedin.ugc.ShareContent":content},
            "visibility":{"com.linkedin.ugc.MemberNetworkVisibility":"PUBLIC"}});
        let response = call_operation(&t, "create_post", arguments, false).await;
        assert_eq!(response["_is_error"], false, "{response}");
        let sent = t.api.requests.recv().await.unwrap();
        let end = sent.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        assert!(String::from_utf8_lossy(&sent[..end]).starts_with("POST /v2/ugcPosts "));
        let body: Value = serde_json::from_slice(&sent[end..]).unwrap();
        assert_eq!(
            body["specificContent"]["com.linkedin.ugc.ShareContent"]["shareMediaCategory"],
            category
        );
    }
    assert!(t.upload.requests.try_recv().is_err());
    t.f.state.db.drop().await.unwrap();
}

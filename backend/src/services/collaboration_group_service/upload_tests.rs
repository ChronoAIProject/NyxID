use super::*;
use crate::services::{assistant_nyxagent as engine, assistant_upload_service as uploads};
use serde_json::{Value, json};

fn png() -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::RgbImage::new(2, 2)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}
fn capabilities() -> Value {
    json!({"protocol":"nyxagent-input-image-v1","input_image":{"version":1,"sources":["nyxid_attachment_url"],"max_images":10,"max_image_bytes":1048576,"max_total_bytes":10485760,"content_types":["image/png"]}})
}
async fn fixture(
    guest: bool,
) -> (
    Fixture,
    AssistantAgent,
    AssistantGroup,
    String,
    runtime::StartedTurn,
) {
    let (f, agent, group) = setup("collaboration_image_input").await;
    let mut group = join(&f, &group, &f.member).await;
    if guest {
        group = change(
            &f,
            &f.admin,
            &group,
            Change::GuestAccess {
                agent_id: agent.id.clone(),
                enabled: true,
                expected_revision: 1,
                request_id: None,
            },
        )
        .await
        .group
        .unwrap();
    } else {
        share(&f, &agent, &[&f.member]).await;
    }
    let image = uploads::upload(
        &f.state.db,
        &f.state.encryption_keys,
        &f.member,
        &group.id,
        "iou.png",
        png(),
    )
    .await
    .unwrap();
    let request = requests::enqueue_with_uploads_ready(
        &f.state.db,
        &f.member,
        &group.id,
        "Read this image",
        Some(std::slice::from_ref(&agent.id)),
        std::slice::from_ref(&image.id),
    )
    .await
    .unwrap();
    let started = runtime::begin_ready(
        &f.state.db,
        &f.state.encryption_keys,
        &f.member,
        &group.id,
        &request.id,
        &agent.id,
    )
    .await
    .unwrap();
    (f, agent, group, image.id, started)
}
async fn items(
    f: &Fixture,
    row: &crate::models::assistant_conversation::AssistantConversation,
) -> Vec<crate::models::assistant_conversation::TurnAttachment> {
    uploads::turn_attachments(
        &f.state.db,
        &row.user_id,
        &row.id,
        &row.active_turn.as_ref().unwrap().turn_id,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn collaboration_guest_images_use_exact_request_urls_and_live_delivery_fences() {
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    for guest in [false, true] {
        let (f, agent, group, image, started) = fixture(guest).await;
        let row = &started.claim.conversation;
        let chat = svc::assistant_acknowledgement_service::for_key(
            &f.state.db,
            &row.user_id,
            Some(&row.credential_api_key_id),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(chat.guest, guest);
        if guest {
            let mut channel_guest = chat.clone();
            channel_guest.collaboration_thread = false;
            assert!(
                uploads::for_chat(&f.state.db, &channel_guest, &image)
                    .await
                    .is_err()
            );
            assert!(
                uploads::list_for_chat(&f.state.db, &channel_guest, None, 50)
                    .await
                    .is_err()
            );
        }
        let input = items(&f, row).await;
        assert_eq!(
            input.len(),
            1,
            "request images must reach both execution lanes"
        );
        let (available, note) = super::uploads::input_for_turn(&f.state.db, row, input)
            .await
            .unwrap();
        assert!(note.is_empty());
        let (parts, omitted) =
            uploads::image_plan(&available, &capabilities(), &f.state.config.base_url);
        assert!(omitted.is_empty());
        assert_eq!(parts,json!([{"type":"input_image","image_url":format!("{}/api/v1/assistant-attachments/{image}/content",f.state.config.base_url.trim_end_matches('/'))}]).as_array().unwrap().clone());
        let (_, router) = crate::routes::build_router_with_state(f.state.clone());
        let router = router.with_state(f.state.clone());
        let fetch = |id: &str| {
            Request::builder()
                .uri(format!("/api/v1/assistant-attachments/{id}/content"))
                .header(
                    "authorization",
                    format!("Bearer {}", started.claim.credential.raw_key.as_str()),
                )
                .body(Body::empty())
                .unwrap()
        };
        let response = router.clone().oneshot(fetch(&image)).await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap()
                .as_ref(),
            png()
        );
        // Public uploads from another person/request and private drafts stay outside this turn.
        for (owner, publish) in [(&f.admin, true), (&f.member, true), (&f.member, false)] {
            let other = uploads::upload(
                &f.state.db,
                &f.state.encryption_keys,
                owner,
                &group.id,
                "other.png",
                png(),
            )
            .await
            .unwrap();
            if publish {
                let request = requests::enqueue_with_uploads_ready(
                    &f.state.db,
                    owner,
                    &group.id,
                    "Another request",
                    Some(std::slice::from_ref(&agent.id)),
                    std::slice::from_ref(&other.id),
                )
                .await
                .unwrap();
                requests::cancel(&f.state.db, owner, &group.id, &request.id)
                    .await
                    .unwrap();
            }
            assert!(
                !router
                    .clone()
                    .oneshot(fetch(&other.id))
                    .await
                    .unwrap()
                    .status()
                    .is_success()
            );
            assert!(
                uploads::read(
                    &f.state.db,
                    &f.state.encryption_keys,
                    &chat,
                    &json!({"attachment_id":other.id})
                )
                .await
                .is_err()
            );
        }
        let listing = uploads::list_for_chat(&f.state.db, &chat, None, 50)
            .await
            .unwrap();
        assert_eq!(listing["attachments"].as_array().unwrap().len(), 1);
        assert_eq!(listing["attachments"][0]["id"], image);
        let stored = f
            .state
            .db
            .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
            .find_one(doc! {"_id":&image})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.get_str("first_used_turn_id").unwrap(),
            row.active_turn.as_ref().unwrap().turn_id
        );
        if guest {
            change(
                &f,
                &f.admin,
                &group,
                Change::GuestAccess {
                    agent_id: agent.id,
                    enabled: false,
                    expected_revision: 2,
                    request_id: None,
                },
            )
            .await;
        } else {
            change(
                &f,
                &f.admin,
                &group,
                Change::Remove {
                    person_id: f.member.clone(),
                },
            )
            .await;
        }
        assert!(
            !router
                .clone()
                .oneshot(fetch(&image))
                .await
                .unwrap()
                .status()
                .is_success()
        );
        assert!(
            uploads::list_for_chat(&f.state.db, &chat, None, 50)
                .await
                .is_err()
        );
        assert!(
            super::uploads::input_for_turn(&f.state.db, row, items(&f, row).await)
                .await
                .is_err()
        );
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn collaboration_guest_image_withholding_and_expiry_are_explicit() {
    let (f, _, group, image, started) = fixture(true).await;
    let row = &started.claim.conversation;
    let original = items(&f, row).await;
    let mut doc = original[0].clone();
    doc.id = uuid::Uuid::new_v4().to_string();
    doc.content_type = "text/plain".into();
    doc.label = "report.txt".into();
    let (_, note) = super::uploads::input_for_turn(&f.state.db, row, vec![doc.clone()])
        .await
        .unwrap();
    assert!(note.contains("not delivered"));
    assert!(note.contains("Do not claim"));
    let chat = svc::assistant_acknowledgement_service::for_key(
        &f.state.db,
        &row.user_id,
        Some(&row.credential_api_key_id),
    )
    .await
    .unwrap()
    .unwrap();
    let document = uploads::upload(
        &f.state.db,
        &f.state.encryption_keys,
        &f.member,
        &group.id,
        "report.txt",
        b"public document".to_vec(),
    )
    .await
    .unwrap();
    assert!(
        uploads::for_chat(&f.state.db, &chat, &document.id)
            .await
            .is_err()
    );
    for cap in [
        Value::Null,
        json!({"protocol":"nyxagent-input-image-v1","input_image":{"version":1,"sources":["nyxid_attachment_url"],"max_images":0}}),
        json!({"protocol":"nyxagent-input-image-v1","input_image":{"version":1,"sources":["nyxid_attachment_url"],"max_images":10,"max_image_bytes":1,"max_total_bytes":1,"content_types":["image/png"]}}),
    ] {
        let (parts, omitted) = uploads::image_plan(&original, &cap, &f.state.config.base_url);
        assert!(parts.is_empty());
        assert_eq!(omitted, vec![image.clone()]);
    }
    f.state.db.collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME).update_one(doc!{"_id":&image},doc!{"$set":{"bound_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::days(500))}}).await.unwrap();
    let (available, note) = super::uploads::input_for_turn(&f.state.db, row, original)
        .await
        .unwrap();
    assert!(available.is_empty());
    assert!(note.contains("expired per retention policy"));
    assert!(matches!(
        uploads::for_chat(&f.state.db, &chat, &image).await,
        Err(AppError::AssistantAttachmentExpired)
    ));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn collaboration_guest_worker_delivers_images_and_emits_capability_or_expiry_notices() {
    use axum::{
        Json, Router,
        body::{Body, to_bytes},
        http::{HeaderMap, Request},
        response::IntoResponse,
        routing::{get, post},
    };
    use std::{sync::Arc, time::Duration};
    use tokio::sync::Mutex;
    use tower::ServiceExt;
    for mode in ["supported", "unsupported", "expired"] {
        let (f, _, _, image, started) = fixture(true).await;
        if mode == "expired" {
            f.state.db.collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME).update_one(doc!{"_id":&image},doc!{"$set":{"bound_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::days(500))}}).await.unwrap();
        }
        let captures = Arc::new(Mutex::new(Vec::<(Value, Vec<u16>)>::new()));
        let output = captures.clone();
        let callback_state = f.state.clone();
        let upstream=Router::new()
            .route("/v1/models",get(||async {Json(json!({"data":[]}))}))
            .route("/v1/capabilities",get(move ||async move {Json(if mode=="supported" {capabilities()}else{Value::Null})}))
            .route("/v1/responses",post(move |headers:HeaderMap,Json(body):Json<Value>| {
                let captures=output.clone();let state=callback_state.clone();
                async move {
                    let mut statuses=Vec::new();
                    let (_,router)=crate::routes::build_router_with_state(state.clone());
                    let router=router.with_state(state);
                    for message in body["input"].as_array().into_iter().flatten() {
                        for part in message["content"].as_array().into_iter().flatten().filter(|p|p["type"]=="input_image") {
                            let url=url::Url::parse(part["image_url"].as_str().unwrap()).unwrap();
                            let response=router.clone().oneshot(Request::builder().uri(url.path()).header("authorization",headers.get("authorization").unwrap()).body(Body::empty()).unwrap()).await.unwrap();
                            let status=response.status().as_u16();
                            let bytes=to_bytes(response.into_body(),1024*1024).await.unwrap();
                            statuses.push(if status==200 && bytes.as_ref()==png(){200}else{status.max(400)});
                        }
                    }
                    captures.lock().await.push((body,statuses));
                    let frame=json!({"type":"response.completed","sequence_number":0,"response":{"id":"resp_1234567890abcdef1234567890abcdef_1234567890abcdef1234567890abcdef","conversation":{"id":"conv_1234567890abcdef1234567890abcdef"},"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Image delivery checked"}]}]}});
                    ([("content-type","text/event-stream")],format!("data: {frame}\n\n")).into_response()
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, upstream).await.unwrap() });
        let mut nyx = crate::models::downstream_service::test_helpers::dummy_service();
        nyx.id = uuid::Uuid::new_v4().to_string();
        nyx.slug = engine::SERVICE_SLUG.into();
        nyx.base_url = format!("http://{address}");
        nyx.auth_method = "none".into();
        nyx.requires_user_credential = false;
        nyx.identity_propagation_mode = "none".into();
        nyx.service_category = "internal".into();
        nyx.forward_access_token = true;
        f.state
            .db
            .collection::<crate::models::downstream_service::DownstreamService>(
                "downstream_services",
            )
            .insert_one(nyx)
            .await
            .unwrap();
        let row = started.claim.conversation;
        let row_id = row.id.clone();
        let auth = crate::handlers::assistant_team::owner_auth(&row.user_id).unwrap();
        let permit = crate::handlers::assistant_team::acquire(
            &f.state,
            crate::handlers::assistant_team::Pool::Channel {
                owner: &row.user_id,
            },
        )
        .await
        .unwrap()
        .unwrap();
        let (sender, _) = tokio::sync::broadcast::channel(256);
        tokio::time::timeout(
            Duration::from_secs(45),
            crate::handlers::assistant_nyxagent::run_turn(
                f.state.clone(),
                auth,
                row,
                started.input,
                started.claim.credential,
                Some(crate::handlers::assistant_nyxagent::SERVER_TURN_POLICY),
                permit,
                svc::assistant_nyxagent_runtime::Events {
                    checkpoint: None,
                    sender,
                    cursor: 0,
                    reset_notified: false,
                },
            ),
        )
        .await
        .expect("guest image worker must settle");
        server.abort();
        let captured = captures.lock().await;
        assert_eq!(captured.len(), 1, "{mode}: worker must call NyxAgent once");
        let (body, statuses) = &captured[0];
        if mode == "supported" {
            assert_eq!(statuses, &vec![200]);
            assert!(
                body.to_string()
                    .contains(&format!("/assistant-attachments/{image}/content"))
            );
        } else {
            assert!(statuses.is_empty());
            assert!(!body.to_string().contains("\"type\":\"input_image\""));
            assert!(
                body.to_string().contains(if mode == "expired" {
                    "expired per retention policy"
                } else {
                    uploads::IMAGE_FALLBACK
                }),
                "{mode}: withheld attachment must be explicit"
            );
        }
        let reply = f
            .state
            .db
            .collection::<crate::models::assistant_message::AssistantMessage>("assistant_messages")
            .find_one(doc! {"conversation_id":&row_id,"role":"assistant"})
            .await
            .unwrap()
            .unwrap();
        assert!(reply.error_code.is_none(), "{mode}: {:?}", reply.error_code);
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn collaboration_guest_image_delete_after_turn_tracks_the_requester() {
    let (f, _, _, image, started) = fixture(true).await;
    let settings = crate::models::assistant_upload_retention::Settings {
        policy: Some(crate::models::assistant_upload_retention::Policy {
            images_delete_after_turn: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    f.state
        .db
        .collection::<crate::models::assistant_upload_retention::Settings>(
            crate::models::platform_settings::COLLECTION_NAME,
        )
        .insert_one(settings)
        .await
        .unwrap();
    let row = &started.claim.conversation;
    let (available, note) = super::uploads::input_for_turn(&f.state.db, row, items(&f, row).await)
        .await
        .unwrap();
    assert_eq!(available.len(), 1);
    assert!(note.is_empty());
    engine::finish_turn(
        &f.state.db,
        row,
        &row.credential_api_key_id,
        &uuid::Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "Done".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    assert!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
            .find_one(doc! {"_id":&image})
            .await
            .unwrap()
            .is_none()
    );
    f.state.db.drop().await.unwrap();
}

//! The post-connect #1974 journey uses the saved bot/route/specialist throughout.
//! CMAEG and Telegram are local HTTP fixtures; native/live delivery is separate.
use super::*;
use crate::models::{assistant_message::AssistantMessage, assistant_upload::AssistantUpload};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};

struct TelegramOrigin(String);

impl Drop for TelegramOrigin {
    fn drop(&mut self) {
        crate::services::telegram_input::TEST_API_BASES
            .lock()
            .unwrap()
            .remove(&self.0);
    }
}

fn input_text(body: &Value) -> &str {
    body["input"]
        .as_str()
        .unwrap_or_else(|| body["input"][0]["content"][0]["text"].as_str().unwrap())
}

fn assert_answer(sse: &str, expected: &str) {
    let frames: Vec<Value> = sse
        .lines()
        .filter_map(|line| {
            line.strip_prefix("data: ")
                .and_then(|data| serde_json::from_str(data).ok())
        })
        .collect();
    let messages: Vec<_> = frames
        .iter()
        .filter(|frame| frame["type"] == "response.output_item.done")
        .collect();
    assert_eq!(messages.len(), 1, "{sse}");
    assert_eq!(messages[0]["item"]["role"], "assistant");
    assert_eq!(
        messages[0]["item"]["content"],
        json!([{"type":"output_text","text":expected}])
    );
    let completed = frames
        .iter()
        .find(|frame| frame["type"] == "response.completed")
        .unwrap();
    assert_eq!(
        completed["response"]["output"],
        json!([messages[0]["item"]])
    );
}

async fn model_for_journey(
    f: &mut Fixture,
    row: &NyxbotChannel,
    questions: [String; 2],
    answers: [String; 2],
    jpeg: Vec<u8>,
) {
    let state = f.state.clone();
    let home_id = f.home.id.clone();
    let specialist_id = f.specialist.id.clone();
    let channel_id = row.id.clone();
    let calls = f.model_calls.clone();
    let router = Router::new().route("/v1/responses", post(move |headers: HeaderMap, Json(mut body): Json<Value>| {
        let (state, home_id, specialist_id, channel_id, calls, questions, answers, jpeg) = (
            state.clone(), home_id.clone(), specialist_id.clone(), channel_id.clone(), calls.clone(),
            questions.clone(), answers.clone(), jpeg.clone(),
        );
        async move {
            let raw = headers["authorization"].to_str().unwrap().strip_prefix("Bearer ").unwrap();
            let (owner, key, credential_id) = key_service::validate_api_key(&state.db, raw).await.unwrap();
            assert_eq!(owner, OWNER);
            let saved = state.db.collection::<AssistantConversation>(CONVERSATIONS)
                .find_one(doc! {"credential_api_key_id":&key.id}).await.unwrap().unwrap();
            assert_eq!(saved.id, home_id, "No NyxBot or new channel-thread fallback");
            assert_eq!(saved.agent_id.as_deref(), Some(specialist_id.as_str()));
            assert!(!saved.guest_turn);
            let active = saved.active_turn.as_ref().unwrap();
            let context = active.channel_context.as_ref().unwrap();
            assert_eq!(context.channel_id, channel_id);
            assert!(!context.guest);
            assert!(context.original_message);
            let photo = input_text(&body).contains(&questions[1]);
            assert!(input_text(&body).contains(&questions[usize::from(photo)]));
            let images: Vec<_> = body["input"].as_array().into_iter().flatten()
                .flat_map(|item| item["content"].as_array().into_iter().flatten())
                .filter(|part| part["type"] == "input_image").collect();
            assert_eq!(images.len(), usize::from(photo), "{body}");
            assert!(!input_text(&body).contains(crate::services::assistant_upload_service::IMAGE_FALLBACK));
            assert!(!input_text(&body).contains(crate::services::assistant_channel_media::FAILED_NOTE));
            let image_id = if photo {
                let image_url = url::Url::parse(images[0]["image_url"].as_str().unwrap()).unwrap();
                let image_id = image_url.path().strip_prefix("/api/v1/assistant-attachments/")
                    .unwrap().strip_suffix("/content").unwrap().to_owned();
                // Consume the actual image URL under the authenticated model thread
                // key while its exact owner turn is live, before returning an answer.
                let auth = crate::mw::auth::api_key_auth_user(&state.db, &key, credential_id, None, None).await.unwrap();
                let response = crate::handlers::assistant_uploads::thread_image(
                    State(state.clone()), auth, Path(image_id.clone()),
                ).await.unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                assert_eq!(response.headers()["content-type"], "image/jpeg");
                let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
                assert_eq!(bytes.as_ref(), jpeg.as_slice(), "The model must receive this photo's exact bytes");
                Some(image_id)
            } else { None };
            body["_test_turn"] = json!(active.turn_id);
            body["_test_channel_context"] = serde_json::to_value(context).unwrap();
            body["_test_image_id"] = json!(image_id);
            calls.lock().await.push(body);
            let session = format!("conv_{}", Uuid::new_v4().simple());
            let response = format!("resp_{}_{}", &session[5..], Uuid::new_v4().simple());
            let output = message_output(&answers[usize::from(photo)]);
            let stream = async_stream::stream! {
                yield Ok::<_, Infallible>(Event::default().data(json!({
                    "type":"response.completed", "sequence_number":1,
                    "response":{"id":response,"conversation":{"id":session},"status":"completed","output":output},
                }).to_string()));
            };
            axum::response::Sse::new(stream).into_response()
        }
    })).route("/v1/capabilities", axum::routing::get(|| async {
        Json(json!({"protocol":"nyxagent-input-image-v1","input_image":{
            "version":1,"sources":["nyxid_attachment_url"],"max_images":1,
            "max_image_bytes":20971520,"max_total_bytes":20971520,"content_types":["image/jpeg"],
        }}))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    f.state
        .db
        .collection::<bson::Document>(SERVICES)
        .update_one(
            doc! {"slug":engine::SERVICE_SLUG},
            doc! {"$set":{"base_url":format!("http://{address}")}},
        )
        .await
        .unwrap();
    f.model.abort();
    f.model = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
}

pub(super) async fn text_and_own_photo(f: &mut Fixture, row: &NyxbotChannel) {
    let _budget = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let unique = Uuid::new_v4().simple().to_string();
    let questions = [
        format!("who are you, text {unique}"),
        format!("Read my own photo {unique}"),
    ];
    let answers = [
        format!("Specialist {} text answer {unique}", f.specialist.id),
        format!("Specialist {} photo answer {unique}", f.specialist.id),
    ];
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95)
        .encode(
            &[240, 30, 10, 10, 30, 240],
            2,
            1,
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    model_for_journey(f, row, questions.clone(), answers.clone(), jpeg.clone()).await;
    let telegram = MockServer::start().await;
    let file_id = format!("own_photo_{unique}");
    let file_path = format!("photos/{unique}.jpg");
    Mock::given(method("GET"))
        .and(path(format!("/bot{BOT_TOKEN}/getFile")))
        .and(query_param("file_id", &file_id))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,
            "result":{"file_path":file_path,"file_size":jpeg.len()}})))
        .expect(1)
        .mount(&telegram)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/file/bot{BOT_TOKEN}/{file_path}")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(jpeg.clone())
                .insert_header("content-type", "image/jpeg"),
        )
        .expect(1)
        .mount(&telegram)
        .await;
    crate::services::telegram_input::TEST_API_BASES
        .lock()
        .unwrap()
        .insert(f.bot.id.clone(), telegram.uri());
    let _origin = TelegramOrigin(f.bot.id.clone());
    feature_flag_service::set_platform_override(
        &f.state.db,
        feature_flag_service::ASSISTANT_CHANNEL_MEDIA_FLAG_KEY,
        &feature_flag_service::FlagTarget::User(OWNER.into()),
        true,
        OWNER,
    )
    .await
    .unwrap();
    let credential = decrypt_agent_key(&f.state, row).await.unwrap().unwrap();
    let mut inputs = Vec::new();
    for (index, question) in questions.iter().enumerate() {
        let number = 3 + index as i64;
        let mut update = json!({"update_id":number,"message":{
            "message_id":number,"chat":{"id":77,"type":"private"},
            "from":{"id":77,"is_bot":false},"text":question,
        }});
        if index == 1 {
            update["message"].as_object_mut().unwrap().remove("text");
            update["message"]["caption"] = json!(question);
            update["message"]["photo"] = json!([
                {"file_id":"unused_thumbnail","file_unique_id":"thumbnail","width":1,"height":1,"file_size":1},
                {"file_id":file_id,"file_unique_id":unique,"width":2,"height":1,"file_size":jpeg.len()},
            ]);
        }
        let (body, id) = provider_event(f, row, update).await;
        let response = respond(&f.state, &credential, &body, &id).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_answer(&body_text(response).await, &answers[index]);
        assert_eq!(f.model_calls.lock().await.len(), index + 1);
        inputs.push((body, id));
    }
    let calls = f.model_calls.lock().await.clone();
    assert_eq!(calls.len(), 2);
    assert_ne!(calls[0]["_test_turn"], calls[1]["_test_turn"]);
    let messages = f
        .state
        .db
        .collection::<AssistantMessage>(crate::models::assistant_message::COLLECTION_NAME);
    for (index, (_, id)) in inputs.iter().enumerate() {
        let event = f
            .state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"channel_id":&row.id,"event_id":id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.status, "completed");
        assert_eq!(event.user_id, OWNER);
        assert_eq!(event.conversation_id.as_deref(), Some(f.home.id.as_str()));
        assert_eq!(
            event.turn_id.as_deref(),
            calls[index]["_test_turn"].as_str()
        );
        assert_eq!(
            calls[index]["_test_channel_context"]["source_event_id"],
            event.id
        );
        let turn_id = event.turn_id.as_deref().unwrap();
        let rows: Vec<AssistantMessage> = messages
            .find(doc! {"turn_id":turn_id})
            .sort(doc! {"seq":1})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(
            rows.len(),
            2,
            "One user message and one specialist answer per turn"
        );
        for message in &rows {
            assert_eq!(message.user_id, OWNER);
            assert_eq!(message.conversation_id, f.home.id);
        }
        assert_eq!(rows[0].role, "user");
        assert_eq!(rows[0].text, questions[index]);
        assert_eq!(rows[0].attachments.len(), index);
        assert_eq!(rows[1].role, "assistant");
        assert_eq!(rows[1].text, answers[index]);
        assert_eq!(rows[1].status, "completed");
        if index == 1 {
            let image = &rows[0].attachments[0];
            assert_eq!(image.content_type, "image/jpeg");
            assert_eq!(
                Some(image.id.as_str()),
                calls[index]["_test_image_id"].as_str()
            );
            let upload = f
                .state
                .db
                .collection::<AssistantUpload>(crate::models::assistant_upload::COLLECTION_NAME)
                .find_one(doc! {"_id":&image.id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(upload.user_id, OWNER);
            assert_eq!(upload.conversation_id, f.home.id);
            assert_eq!(upload.message_id.as_deref(), Some(rows[0].id.as_str()));
            assert_eq!(upload.size, jpeg.len() as i64);
        }
    }
    let saved = engine::get(&f.state.db, OWNER, &f.home.id).await.unwrap();
    assert!(saved.active_turn.is_none());
    assert_eq!(saved.agent_id, row.agent_id);
    let reply = saved.reply_channel.unwrap();
    assert_eq!(reply.nyxbot_channel_id, row.id);
    assert_eq!(reply.partition, PARTITION);
    assert_eq!(reply.platform, "telegram");
    // Replay the same gateway commands. Cached output may be repeated to its
    // observer; neither source may create a second execution or transcript pair.
    let before = messages.count_documents(doc! {}).await.unwrap();
    for (body, id) in &inputs {
        let response = respond(&f.state, &credential, body, id).await;
        assert_eq!(response.status(), StatusCode::OK);
        body_text(response).await;
    }
    assert_eq!(f.model_calls.lock().await.len(), 2);
    assert_eq!(messages.count_documents(doc! {}).await.unwrap(), before);
    telegram.verify().await;
    assert_eq!(telegram.received_requests().await.unwrap().len(), 2);
}

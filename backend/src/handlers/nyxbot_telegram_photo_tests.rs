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
    // This provider emits JSON data records, with no [DONE] sentinel. Ignore
    // comments/other SSE fields, but never discard malformed data records.
    let frames: Vec<Value> = sse
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|data| {
            serde_json::from_str(data.strip_prefix(' ').unwrap_or(data))
                .expect("NyxBot provider SSE data must be JSON")
        })
        .collect();
    assert!(frames.iter().all(|frame| matches!(
        frame["type"].as_str(),
        Some("response.created" | "response.output_item.done" | "response.completed")
    )));
    let created: Vec<_> = frames
        .iter()
        .filter(|frame| frame["type"] == "response.created")
        .collect();
    let messages: Vec<_> = frames
        .iter()
        .filter(|frame| frame["type"] == "response.output_item.done")
        .collect();
    let completed: Vec<_> = frames
        .iter()
        .filter(|frame| frame["type"] == "response.completed")
        .collect();
    assert_eq!(created.len(), 1);
    assert_eq!(messages.len(), 1);
    assert_eq!(completed.len(), 1);
    assert_eq!(frames.first(), Some(created[0]));
    assert_eq!(frames.last(), Some(completed[0]));
    assert!(
        completed[0]["response"]["id"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
    assert_eq!(created[0]["response"]["id"], completed[0]["response"]["id"]);
    assert_eq!(completed[0]["response"]["status"], "completed");
    assert_eq!(messages[0]["output_index"], 0);
    assert_eq!(messages[0]["item"]["type"], "message");
    assert_eq!(messages[0]["item"]["role"], "assistant");
    assert_eq!(
        messages[0]["item"]["content"],
        json!([{"type":"output_text","text":expected}])
    );
    assert_eq!(
        completed[0]["response"]["output"],
        json!([messages[0]["item"]])
    );
}

#[derive(Clone)]
struct ModelObservation {
    turn_id: String,
    source_event_id: String,
    image_id: Option<String>,
}

type Observations = Arc<Mutex<Vec<ModelObservation>>>;

struct JourneyModel {
    state: AppState,
    private_router: Router,
    home_id: String,
    specialist_id: String,
    channel_id: String,
    questions: [String; 2],
    answers: [String; 2],
    jpeg: Vec<u8>,
    observations: Observations,
}

impl JourneyModel {
    async fn read_photo(
        &self,
        authorization: &axum::http::HeaderValue,
        image_part: &Value,
        active: &crate::models::assistant_conversation::ActiveTurn,
    ) -> String {
        use tower::ServiceExt;
        assert_eq!(active.attachments.len(), 1);
        let image = &active.attachments[0];
        assert_eq!(image.content_type, "image/jpeg");
        let expected_url = format!(
            "{}/api/v1/assistant-attachments/{}/content",
            self.state.config.base_url.trim_end_matches('/'),
            image.id
        );
        let emitted_url = image_part["image_url"]
            .as_str()
            .expect("image URL required");
        // Compare the complete emitted URL, including origin, path, query and
        // fragment. Do not print URLs: a regression could place credentials there.
        assert!(
            emitted_url == expected_url,
            "Image URL must exactly match the configured base and this turn's attachment"
        );
        let mut authorization = authorization.clone();
        authorization.set_sensitive(true);
        let request = axum::http::Request::builder()
            .method("GET")
            .uri(emitted_url)
            .header("authorization", authorization)
            .body(Body::empty())
            .expect("valid model image request");
        // Use the received thread bearer through the production route and all
        // authentication/turn middleware; the fixture grants no extra authority.
        let response = self.private_router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "image/jpeg");
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(
            bytes.as_ref(),
            self.jpeg.as_slice(),
            "The model must receive this photo's exact bytes"
        );
        image.id.clone()
    }
}

async fn model_response(
    State(model): State<Arc<JourneyModel>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let authorization = headers
        .get("authorization")
        .expect("model thread bearer required");
    let raw = authorization
        .to_str()
        .unwrap()
        .strip_prefix("Bearer ")
        .unwrap();
    let (owner, key, _) = key_service::validate_api_key(&model.state.db, raw)
        .await
        .unwrap();
    assert_eq!(owner, OWNER);
    let saved = model
        .state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"credential_api_key_id":&key.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.id, model.home_id, "Unexpected specialist home");
    assert_eq!(
        saved.agent_id.as_deref(),
        Some(model.specialist_id.as_str())
    );
    assert!(!saved.guest_turn);
    let active = saved.active_turn.as_ref().unwrap();
    let context = active.channel_context.as_ref().unwrap();
    assert_eq!(context.channel_id, model.channel_id);
    assert!(!context.guest);
    assert!(context.original_message);
    let photo = input_text(&body).contains(&model.questions[1]);
    let index = usize::from(photo);
    assert!(input_text(&body).contains(&model.questions[index]));
    let images: Vec<_> = body["input"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter(|part| part["type"] == "input_image")
        .collect();
    assert_eq!(images.len(), index);
    assert!(!input_text(&body).contains(crate::services::assistant_upload_service::IMAGE_FALLBACK));
    assert!(!input_text(&body).contains(crate::services::assistant_channel_media::FAILED_NOTE));
    let image_id = if photo {
        Some(model.read_photo(authorization, images[0], active).await)
    } else {
        None
    };
    model.observations.lock().await.push(ModelObservation {
        turn_id: active.turn_id.clone(),
        source_event_id: context.source_event_id.clone(),
        image_id,
    });
    let session = format!("conv_{}", Uuid::new_v4().simple());
    let response = format!("resp_{}_{}", &session[5..], Uuid::new_v4().simple());
    let output = message_output(&model.answers[index]);
    let stream = async_stream::stream! {
        yield Ok::<_, Infallible>(Event::default().data(json!({
            "type":"response.completed", "sequence_number":1,
            "response":{"id":response,"conversation":{"id":session},"status":"completed","output":output},
        }).to_string()));
    };
    axum::response::Sse::new(stream).into_response()
}

async fn model_for_journey(
    f: &mut Fixture,
    row: &NyxbotChannel,
    questions: [String; 2],
    answers: [String; 2],
    jpeg: Vec<u8>,
) -> Observations {
    let observations = Observations::default();
    let (_, private) = crate::routes::build_router_with_state(f.state.clone());
    let model = Arc::new(JourneyModel {
        state: f.state.clone(),
        private_router: private.with_state(f.state.clone()),
        home_id: f.home.id.clone(),
        specialist_id: f.specialist.id.clone(),
        channel_id: row.id.clone(),
        questions,
        answers,
        jpeg,
        observations: observations.clone(),
    });
    let router = Router::new()
        .route("/v1/responses", post(model_response))
        .route("/v1/capabilities", axum::routing::get(|| async {
            Json(json!({"protocol":"nyxagent-input-image-v1","input_image":{
                "version":1,"sources":["nyxid_attachment_url"],"max_images":1,
                "max_image_bytes":20971520,"max_total_bytes":20971520,"content_types":["image/jpeg"],
            }}))
        })).with_state(model);
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
    observations
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
    let observations =
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
        assert_eq!(observations.lock().await.len(), index + 1);
        inputs.push((body, id));
    }
    let calls = observations.lock().await.clone();
    assert_eq!(calls.len(), 2);
    assert_ne!(calls[0].turn_id, calls[1].turn_id);
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
            Some(calls[index].turn_id.as_str())
        );
        assert_eq!(calls[index].source_event_id, event.id);
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
            assert_eq!(Some(image.id.as_str()), calls[index].image_id.as_deref());
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
    assert_eq!(observations.lock().await.len(), 2);
    assert_eq!(messages.count_documents(doc! {}).await.unwrap(), before);
    telegram.verify().await;
    assert_eq!(telegram.received_requests().await.unwrap().len(), 2);
}

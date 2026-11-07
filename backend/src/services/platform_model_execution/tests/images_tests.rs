use super::*;
use crate::models::platform_model::Modality as ModelModality;
use base64::{Engine, engine::general_purpose::STANDARD};

fn fixture_value(plan: &Plan) -> Value {
    serde_json::from_str(certification::image_fixture(&plan.routes[0].target).unwrap()).unwrap()
}
pub(super) fn image_plan(plan: &mut Plan, split: bool) -> Request {
    let fixture = certification::image_fixture(&plan.routes[0].target).unwrap();
    let value: Value = serde_json::from_str(fixture).unwrap();
    plan.model
        .capabilities
        .input_modalities
        .push(ModelModality::Image);
    let mut tariff = plan.tariff.tariff().clone();
    tariff.contract.fixture_sha256 = certification::image_digest(&plan.routes[0].target).unwrap();
    tariff.bounds.max_images = 10;
    tariff.bounds.max_request_bytes = images::MAX_REQUEST as i64;
    if split {
        tariff.contract.input_modalities =
            vec![crate::models::platform_model_tariff::Modality::Image];
    }
    tariff.components = billing::model_usage::selectors(&tariff.contract)
        .unwrap()
        .into_iter()
        .enumerate()
        .map(|(i, selector)| TariffComponent {
            key: format!("unit{i}"),
            metric: billing::model_usage::metric(selector),
            selector,
            credits_per_unit: "0".into(),
            explicit_free: true,
            max_quantity: if selector == Selector::Requests {
                1
            } else {
                1024
            },
            lago_metric_code: String::new(),
        })
        .collect();
    billing::model_tariffs::validate(&mut tariff, &plan.offering.slug).unwrap();
    plan.tariff = billing::model_tariffs::select(
        &plan.offering,
        &plan.model.id,
        CredentialClass::NyxidManagedMaster,
        &BTreeMap::new(),
        &[tariff],
        &HashMap::new(),
        Utc::now(),
        900,
    )
    .unwrap();
    Request::parse(
        &http::Method::POST,
        "chat/completions",
        value["request"].clone(),
    )
    .unwrap()
}
async fn save_image_plan(state: &crate::AppState, plan: &Plan) {
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    state
        .db
        .collection::<ModelTariff>(crate::models::platform_model_tariff::COLLECTION_NAME)
        .replace_one(doc! {"_id":&plan.tariff.tariff().id}, plan.tariff.tariff())
        .await
        .unwrap();
}
fn body_with_uri(uri: &str) -> Value {
    json!({"model":"nyxid/gpt-5","max_tokens":128,"messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":uri}}]}]})
}
fn parse(body: Value) -> AppResult<Request> {
    Request::parse(&http::Method::POST, "chat/completions", body)
}
fn raster(format: image::ImageFormat, width: u32, height: u32) -> String {
    let img = image::DynamicImage::ImageRgb8(image::RgbImage::new(width, height));
    let mut bytes = std::io::Cursor::new(Vec::new());
    img.write_to(&mut bytes, format).unwrap();
    let mime = match format {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::WebP => "image/webp",
        _ => unreachable!(),
    };
    format!("data:{mime};base64,{}", STANDARD.encode(bytes.into_inner()))
}

#[test]
fn image_parser_accepts_static_formats_and_preserves_order_without_resizing() {
    for format in [
        image::ImageFormat::Png,
        image::ImageFormat::Jpeg,
        image::ImageFormat::WebP,
    ] {
        let uri = raster(format, 3, 2);
        let mut body = body_with_uri(&uri);
        body["messages"][0]["content"]
            .as_array_mut()
            .unwrap()
            .insert(0, json!({"type":"text","text":"before"}));
        body["messages"][0]["content"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"text","text":"after"}));
        let request = parse(body.clone()).unwrap();
        assert_eq!(request.body, body);
        assert_eq!(request.images.count, 1);
        assert_eq!(request.images.pixels, 6);
    }
}
#[test]
fn image_parser_rejects_urls_files_ids_spoofing_detail_and_malformed_bytes() {
    let uri = raster(image::ImageFormat::Png, 1, 1);
    for bad in [
        "https://example.invalid/a.png".to_owned(),
        "file:///tmp/a.png".into(),
        "file-provider-id".into(),
        uri.replace("image/png", "image/jpeg"),
        uri.replace("image/png", "image/gif"),
        "data:image/png;base64,AAAA".into(),
        "data:image/png;base64,@@@=".into(),
        "data:image/svg+xml;base64,AAAA".into(),
    ] {
        assert!(parse(body_with_uri(&bad)).is_err());
    }
    for detail in ["low", "high", "original"] {
        let mut b = body_with_uri(&uri);
        b["messages"][0]["content"][0]["image_url"]["detail"] = json!(detail);
        assert!(parse(b).is_err());
    }
    for role in ["system", "developer", "assistant", "tool"] {
        let mut b = body_with_uri(&uri);
        b["messages"][0]["role"] = json!(role);
        assert!(parse(b).is_err());
    }
    let mut b = body_with_uri(&uri);
    b["messages"][0]["content"][0]["image_url"]["file_id"] = json!("file-private");
    assert!(parse(b).is_err());
    let mut bytes = STANDARD.decode(uri.split_once(',').unwrap().1).unwrap();
    bytes.truncate(bytes.len() - 4);
    assert!(
        parse(body_with_uri(&format!(
            "data:image/png;base64,{}",
            STANDARD.encode(bytes)
        )))
        .is_err()
    );
}
#[test]
fn image_count_and_text_limits_apply_to_whole_history() {
    let uri = raster(image::ImageFormat::Png, 1, 1);
    let message = body_with_uri(&uri)["messages"][0].clone();
    let mut body = body_with_uri(&uri);
    body["messages"] = json!(vec![message.clone(); 10]);
    assert_eq!(parse(body.clone()).unwrap().images.count, 10);
    body["messages"].as_array_mut().unwrap().push(message);
    assert!(parse(body).is_err());
    let mut body = body_with_uri(&uri);
    body["messages"]
        .as_array_mut()
        .unwrap()
        .insert(0, json!({"role":"system","content":"x".repeat(1024*1024)}));
    // Refuse oversized non-image content before reserving decoder memory or
    // allocating validation copies, rather than waiting for tariff selection.
    assert!(parse(body).is_err());
}
#[test]
fn image_animation_bombs_and_file_limits_are_refused() {
    let uri = raster(image::ImageFormat::Png, 1, 1);
    let png = STANDARD.decode(uri.split_once(',').unwrap().1).unwrap();
    for kind in [b"acTL", b"fcTL", b"fdAT"] {
        let mut bytes = png.clone();
        let chunk = [0, 0, 0, 0, kind[0], kind[1], kind[2], kind[3], 0, 0, 0, 0];
        bytes.splice(33..33, chunk);
        assert!(
            parse(body_with_uri(&format!(
                "data:image/png;base64,{}",
                STANDARD.encode(bytes)
            )))
            .is_err()
        );
    }
    // Oversized IHDR is refused before pixel allocation (CRC deliberately
    // irrelevant to the refusal); full decoders also reject malformed CRCs.
    let mut bomb = png.clone();
    bomb[16..20].copy_from_slice(&100_000_u32.to_be_bytes());
    bomb[20..24].copy_from_slice(&100_000_u32.to_be_bytes());
    assert!(
        parse(body_with_uri(&format!(
            "data:image/png;base64,{}",
            STANDARD.encode(bomb)
        )))
        .is_err()
    );
    let webp = raster(image::ImageFormat::WebP, 1, 1);
    let mut bytes = STANDARD.decode(webp.split_once(',').unwrap().1).unwrap();
    bytes.extend_from_slice(b"ANIM\0\0\0\0");
    let size = (bytes.len() - 8) as u32;
    bytes[4..8].copy_from_slice(&size.to_le_bytes());
    assert!(
        parse(body_with_uri(&format!(
            "data:image/webp;base64,{}",
            STANDARD.encode(bytes)
        )))
        .is_err()
    );
    assert!(
        parse(body_with_uri(&format!(
            "data:image/png;base64,{}",
            STANDARD.encode(vec![0; images::MAX_FILE + 1])
        )))
        .is_err()
    );
}
#[test]
fn image_certification_preserves_text_digests_and_refuses_unknown_models_or_maas() {
    for (native, vertex) in [
        (Native::Gemini, true),
        (Native::Claude, true),
        (Native::Claude, false),
        (Native::Responses, false),
    ] {
        let (mut plan, text) = configuration(native, vertex);
        assert!(
            adapters::prepare(&plan.model, &plan.routes[0], plan.tariff.tariff(), &text).is_ok()
        );
        let legacy = plan.tariff.tariff().clone();
        let image = image_plan(&mut plan, native == Native::Gemini);
        assert!(adapters::prepare(&plan.model, &plan.routes[0], &legacy, &image).is_err());
        assert!(
            adapters::prepare(&plan.model, &plan.routes[0], plan.tariff.tariff(), &image).is_ok()
        );
        let mut unknown = plan.routes[0].clone();
        match &mut unknown.target {
            RouteTarget::VertexPublisher { native_model, .. }
            | RouteTarget::DirectProvider { native_model, .. } => {
                *native_model = "uncertified-model".into()
            }
            _ => unreachable!(),
        }
        assert!(adapters::prepare(&plan.model, &unknown, plan.tariff.tariff(), &image).is_err());
        let mut small = plan.tariff.tariff().clone();
        small.bounds.max_images = 0;
        assert!(image.validate_bounds(&plan.model, &small).is_err());
        small.bounds.max_images = 10;
        small.bounds.max_request_bytes = 10;
        assert!(image.validate_bounds(&plan.model, &small).is_err());
        let limits =
            images::public_limits(&plan.model, &plan.routes, plan.tariff.tariff()).unwrap();
        assert_eq!(
            limits.accounting,
            if native == Native::Gemini {
                "exact_image_and_cache_partition"
            } else {
                "included_in_input_tokens"
            }
        );
        let json = serde_json::to_string(&limits).unwrap();
        assert!(!json.contains("private"));
        assert!(!json.contains("fixture"));
    }
    let (mut plan, _) = configuration(Native::GptOss, true);
    plan.model
        .capabilities
        .input_modalities
        .push(ModelModality::Image);
    let request = parse(body_with_uri(&raster(image::ImageFormat::Png, 1, 1))).unwrap();
    assert!(
        adapters::prepare(&plan.model, &plan.routes[0], plan.tariff.tariff(), &request).is_err()
    );
}
#[tokio::test]
async fn images_all_four_transports_send_exact_ordered_native_parts_and_meter_once() {
    for (native, vertex) in [
        (Native::Gemini, true),
        (Native::Claude, true),
        (Native::Claude, false),
        (Native::Responses, false),
    ] {
        let (state, mut plan, _) = fixture(native, vertex).await;
        let request = image_plan(&mut plan, native == Native::Gemini);
        save_image_plan(&state, &plan).await;
        let value = fixture_value(&plan);
        let server = MockServer::start().await;
        token_mock(&server).await;
        let prepared =
            adapters::prepare(&plan.model, &plan.routes[0], plan.tariff.tariff(), &request)
                .unwrap();
        let parts = match native {
            Native::Gemini => &prepared.body["contents"][0]["parts"],
            Native::Claude => &prepared.body["messages"][0]["content"],
            Native::Responses => &prepared.body["input"][0]["content"],
            _ => unreachable!(),
        };
        let (mime, data) = images::data_uri(&request.body["messages"][0]["content"][1]).unwrap();
        let expected = match native {
            Native::Gemini => {
                json!([{"text":"Describe this image."},{"inlineData":{"mimeType":mime,"data":data}},{"text":"Be brief."}])
            }
            Native::Claude => {
                json!([{"type":"text","text":"Describe this image."},{"type":"image","source":{"type":"base64","media_type":mime,"data":data}},{"type":"text","text":"Be brief."}])
            }
            Native::Responses => {
                json!([{"type":"input_text","text":"Describe this image."},{"type":"input_image","image_url":request.body["messages"][0]["content"][1]["image_url"]["url"],"detail":"auto"},{"type":"input_text","text":"Be brief."}])
            }
            _ => unreachable!(),
        };
        assert_eq!(*parts, expected);
        Mock::given(method("POST"))
            .and(path_regex("^/v1/.*"))
            .and(wiremock::matchers::body_json(&prepared.body))
            .respond_with(ResponseTemplate::new(200).set_body_json(value["response"].clone()))
            .expect(1)
            .mount(&server)
            .await;
        let runtime = Runtime::mocked(&server.uri());
        let context = context(&state, &runtime, &Allowed);
        let accepted = dispatch(&context, &plan, &request).await.unwrap();
        let public = consume(&context, &plan, accepted, None).await.unwrap();
        assert_eq!(public["model"], plan.model.public_id);
        assert_eq!(
            state
                .db
                .collection::<Document>("usage_meter")
                .count_documents(doc! {})
                .await
                .unwrap(),
            plan.tariff.tariff().components.len() as u64
        );
        server.verify().await;
    }
}
#[test]
fn images_gemini_partition_requires_exact_intersection_and_aggregate_remains_valid() {
    let (mut plan, _) = configuration(Native::Gemini, true);
    drop(image_plan(&mut plan, true));
    let value = fixture_value(&plan);
    let evidence = adapters::evidence(Native::Gemini, &value["response"]["usageMetadata"]).unwrap();
    let normalized =
        billing::model_usage::normalize(&plan.tariff.tariff().contract, &evidence).unwrap();
    assert_eq!(normalized[&Selector::Input], 600);
    assert_eq!(normalized[&Selector::ImageInput], 200);
    assert_eq!(normalized[&Selector::CacheRead], 100);
    assert_eq!(
        normalized[&Selector::CachedModality {
            modality: crate::models::platform_model_tariff::Modality::Image
        }],
        100
    );
    let mut missing = evidence;
    missing.cached_image = None;
    assert!(plan.tariff.usage(&missing).is_err());
    let mut aggregate = plan.tariff.tariff().contract.clone();
    aggregate.input_modalities.clear();
    assert_eq!(
        billing::model_usage::normalize(&aggregate, &missing).unwrap()[&Selector::Input],
        800
    );
}
#[tokio::test]
async fn image_readiness_refuses_before_credentials_and_funding() {
    let (state, mut plan, _) = fixture(Native::Responses, false).await;
    let request = image_plan(&mut plan, false);
    // Even corrupt/missing credential material must not be consulted first.
    state
        .db
        .collection::<Document>("downstream_services")
        .delete_one(doc! {"_id":&plan.routes[0].credential_service_id})
        .await
        .unwrap();
    let server = MockServer::start().await;
    let runtime = Runtime::mocked(&server.uri());
    let context = context(&state, &runtime, &Allowed);
    assert!(matches!(
        dispatch(&context, &plan, &request).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    assert_eq!(
        state
            .db
            .collection::<Document>("usage_meter")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}
#[tokio::test]
async fn images_missing_evidence_or_cancellation_retains_forwarded_holds_and_no_fallback() {
    for cancelled in [false, true] {
        let (state, mut plan, _) = fixture(Native::Responses, false).await;
        let request = image_plan(&mut plan, false);
        save_image_plan(&state, &plan).await;
        let mut backup = plan.routes[0].clone();
        backup.id = uuid::Uuid::new_v4().to_string();
        plan.routes.push(backup);
        let mut value = fixture_value(&plan)["response"].clone();
        value.as_object_mut().unwrap().remove("usage");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(value))
            .expect(1)
            .mount(&server)
            .await;
        let runtime = Runtime::mocked(&server.uri());
        let context = context(&state, &runtime, &Allowed);
        let accepted = dispatch(&context, &plan, &request).await.unwrap();
        if cancelled {
            context.cancel.cancel();
        }
        assert!(consume(&context, &plan, accepted, None).await.is_err());
        assert_eq!(recover(&state.db).await.unwrap(), 0);
        let rows: Vec<UsageMeterRow> = state
            .db
            .collection("usage_meter")
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert!(!rows.is_empty());
        assert!(
            rows.iter()
                .all(|r| r.forwarded && !r.released && r.quantity.is_none())
        );
        assert_eq!(diagnostics(&state.db).await.unwrap().len(), 1);
        server.verify().await;
    }
}

#[tokio::test]
async fn image_streams_validate_complete_evidence_before_public_done() {
    for (native, vertex) in [
        (Native::Gemini, true),
        (Native::Claude, true),
        (Native::Claude, false),
        (Native::Responses, false),
    ] {
        let (state, mut plan, _) = fixture(native, vertex).await;
        let mut request = image_plan(&mut plan, native == Native::Gemini);
        request.streaming = true;
        request.body["stream"] = json!(true);
        save_image_plan(&state, &plan).await;
        let value = fixture_value(&plan);
        let sse = value["stream_events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| format!("data: {e}\n\n"))
            .collect::<String>();
        let mut decoder = stream::Decoder::new(native);
        for byte in sse.bytes() {
            decoder.push(&[byte]).unwrap();
        }
        let (_, evidence) = decoder.finish(true, "public-id").unwrap();
        assert!(plan.tariff.usage(&evidence).is_ok());
        let server = MockServer::start().await;
        token_mock(&server).await;
        Mock::given(method("POST"))
            .and(path_regex("^/v1/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
            .expect(1)
            .mount(&server)
            .await;
        let runtime = Runtime::mocked(&server.uri());
        let context = context(&state, &runtime, &Allowed);
        let accepted = dispatch(&context, &plan, &request).await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        consume(&context, &plan, accepted, Some(&tx)).await.unwrap();
        drop(tx);
        let mut output = String::new();
        while let Some(chunk) = rx.recv().await {
            output.push_str(std::str::from_utf8(&chunk.unwrap()).unwrap());
        }
        assert!(!output.contains("private"));
        server.verify().await;
    }
}
#[tokio::test]
async fn images_zero_consumption_can_fallback_without_double_charge() {
    let (state, mut plan, _) = fixture(Native::Responses, false).await;
    let request = image_plan(&mut plan, false);
    save_image_plan(&state, &plan).await;
    let mut backup = plan.routes[0].clone();
    backup.id = uuid::Uuid::new_v4().to_string();
    plan.routes.push(backup);
    let value = fixture_value(&plan);
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(429).set_body_json(value["zero_refusal"].clone()))
        .up_to_n_times(1)
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(value["response"].clone()))
        .with_priority(2)
        .expect(1)
        .mount(&server)
        .await;
    let runtime = Runtime::mocked(&server.uri());
    let context = context(&state, &runtime, &Allowed);
    let accepted = dispatch(&context, &plan, &request).await.unwrap();
    consume(&context, &plan, accepted, None).await.unwrap();
    assert_eq!(state.db.collection::<Document>("usage_meter").count_documents(doc!{"last_error":"model_zero_consumption_refusal","released":true,"forwarded":false}).await.unwrap(),plan.tariff.tariff().components.len() as u64);
    server.verify().await;
}
#[test]
fn image_input_survives_tool_continuations_and_same_model_claude_transport_fallback() {
    for (native, vertex) in [
        (Native::Gemini, true),
        (Native::Claude, true),
        (Native::Responses, false),
    ] {
        let (mut plan, _) = configuration(native, vertex);
        let request = image_plan(&mut plan, false);
        let mut body = request.body.clone();
        drop(request);
        body["messages"].as_array_mut().unwrap().extend([
            json!({"role":"assistant","content":null,"tool_calls":[{"type":"function","id":"call-1","function":{"name":"lookup","arguments":"{}"}}]}),
            json!({"role":"tool","tool_call_id":"call-1","content":"done"}),
            json!({"role":"user","content":"Answer using the original image."}),
        ]);
        let request = parse(body).unwrap();
        let translated =
            adapters::prepare(&plan.model, &plan.routes[0], plan.tariff.tariff(), &request)
                .unwrap();
        assert!(
            translated.body.to_string().contains("base64")
                || translated.body.to_string().contains("inlineData")
        );
        assert!(translated.body.to_string().contains("done"));
        if native == Native::Claude {
            let (direct, _) = configuration(Native::Claude, false);
            assert!(
                adapters::prepare(
                    &plan.model,
                    &direct.routes[0],
                    plan.tariff.tariff(),
                    &request
                )
                .is_ok()
            );
        }
    }
}
#[test]
fn image_request_and_translated_bytes_obey_ingress_and_tariff_limits() {
    let (mut plan, _) = configuration(Native::Responses, false);
    let request = image_plan(&mut plan, false).with_ingress_limit(32);
    assert!(
        adapters::prepare(&plan.model, &plan.routes[0], plan.tariff.tariff(), &request).is_err()
    );
    let too_large = json!({"messages":[],"x":"a".repeat(images::MAX_REQUEST)});
    assert!(images::json_size(&too_large).is_err());
    assert!(images::reserve(images::MAX_REQUEST + 1).is_err());
}

#[tokio::test]
async fn image_planner_and_public_retail_limits_require_live_certification_and_readiness() {
    use crate::handlers::platform_models::{ListQuery, public_list};
    use axum::{
        Json,
        extract::{Query, State},
    };
    let (state, mut plan, _) = fixture(Native::Gemini, true).await;
    let request = image_plan(&mut plan, true);
    save_image_plan(&state, &plan).await;
    let actor = "00000000-0000-4000-8000-000000000001";
    state
        .db
        .collection::<crate::models::user::User>("users")
        .insert_one(test_utils::test_user(
            actor,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let routing = RoutingRevision {
        id: uuid::Uuid::new_v4().to_string(),
        model_id: plan.model.id.clone(),
        model_revision: plan.model.revision,
        routes: plan.routes.clone(),
        created_at: Utc::now(),
    };
    plan.model.routing_revision_id = Some(routing.id.clone());
    state
        .db
        .collection::<RoutingRevision>(ROUTING_COLLECTION)
        .insert_one(routing)
        .await
        .unwrap();
    state
        .db
        .collection::<PlatformModel>(crate::models::platform_model::COLLECTION_NAME)
        .insert_one(&plan.model)
        .await
        .unwrap();
    state.db.collection::<Document>("feature_flag_overrides").insert_one(doc!{"_id":uuid::Uuid::new_v4().to_string(),"org_user_id":bson::Bson::Null,"flag_key":registry::FLAG,"target_kind":"global","target_key":bson::Bson::Null,"enabled":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),"updated_by":actor}).await.unwrap();
    let loaded = Plan::load(&state.db, actor, &plan.model.public_id, None, 900, &request)
        .await
        .unwrap();
    assert_eq!(loaded.routes.len(), 1);
    let Json(public) = public_list(
        State(state.clone()),
        test_utils::test_auth_user(actor),
        Query(ListQuery {
            after: None,
            limit: None,
        }),
    )
    .await
    .unwrap();
    let value = serde_json::to_value(public).unwrap();
    assert_eq!(
        value["models"][0]["retail_prices"][0]["image_input"]["accounting"],
        "exact_image_and_cache_partition"
    );
    assert_eq!(
        value["models"][0]["retail_prices"][0]["image_input"]["max_request_bytes"],
        state.config.llm_max_body_size
    );
    assert!(!value.to_string().contains("private-project"));
    assert!(
        !value
            .to_string()
            .contains(&plan.routes[0].credential_service_id)
    );
    assert_eq!(
        state
            .db
            .collection::<Document>("usage_meter")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    drop(request);
    // A digest cannot certify a different modality partition on Responses/Claude.
    let (mut claude, _) = configuration(Native::Claude, false);
    let claude_request = image_plan(&mut claude, false);
    let mut wrong = claude.tariff.tariff().clone();
    wrong.contract.input_modalities = vec![crate::models::platform_model_tariff::Modality::Image];
    assert!(adapters::prepare(&claude.model, &claude.routes[0], &wrong, &claude_request).is_err());
}

#[test]
fn image_pixel_total_and_allocation_bounds_refuse_valid_compressed_bombs() {
    // A valid, highly compressible raster still has to fit decoded dimensions.
    let uri = raster(image::ImageFormat::Png, 8001, 4000);
    assert!(parse(body_with_uri(&uri)).is_err());
    let uri = raster(image::ImageFormat::Png, 8000, 4000);
    let mut body = body_with_uri(&uri);
    let part = body["messages"][0]["content"][0].clone();
    body["messages"][0]["content"] = json!([part.clone(), part.clone()]);
    {
        let request = parse(body.clone()).unwrap();
        assert_eq!(request.images.pixels, 64_000_000);
    }
    body["messages"][0]["content"]
        .as_array_mut()
        .unwrap()
        .push(part);
    assert!(parse(body).is_err());
}

fn padded_png(bytes: usize) -> String {
    let uri = raster(image::ImageFormat::Png, 1, 1);
    let mut png = STANDARD.decode(uri.split_once(',').unwrap().1).unwrap();
    let extra = bytes - png.len() - 12;
    let mut chunk = Vec::with_capacity(extra + 12);
    chunk.extend_from_slice(&(extra as u32).to_be_bytes());
    chunk.extend_from_slice(b"npAD");
    chunk.resize(extra + 8, 0);
    let table: Vec<u32> = (0_u32..256)
        .map(|mut n| {
            for _ in 0..8 {
                n = if n & 1 == 1 {
                    0xedb88320 ^ (n >> 1)
                } else {
                    n >> 1
                };
            }
            n
        })
        .collect();
    let mut crc = 0xffff_ffff;
    for byte in &chunk[4..] {
        crc = table[((crc ^ u32::from(*byte)) & 255) as usize] ^ (crc >> 8);
    }
    chunk.extend_from_slice(&(!crc).to_be_bytes());
    png.splice(33..33, chunk);
    assert_eq!(png.len(), bytes);
    format!("data:image/png;base64,{}", STANDARD.encode(png))
}
#[test]
fn image_file_and_total_bytes_are_enforced_at_exact_boundaries() {
    let uri = padded_png(images::MAX_FILE);
    let part = body_with_uri(&uri)["messages"][0]["content"][0].clone();
    let mut body = body_with_uri(&uri);
    body["messages"][0]["content"] = json!([part.clone(), part]);
    {
        let request = parse(body.clone()).unwrap();
        assert_eq!(request.images.bytes, images::MAX_FILES);
    }
    body["messages"][0]["content"].as_array_mut().unwrap().push(
        body_with_uri(&raster(image::ImageFormat::Png, 1, 1))["messages"][0]["content"][0].clone(),
    );
    assert!(parse(body).is_err());
    assert!(parse(body_with_uri(&padded_png(images::MAX_FILE + 1))).is_err());
    let (mut plan, _) = configuration(Native::Claude, false);
    drop(image_plan(&mut plan, false));
    let request = parse(body_with_uri(&padded_png(5 * 1024 * 1024 + 1))).unwrap();
    assert!(
        adapters::prepare(&plan.model, &plan.routes[0], plan.tariff.tariff(), &request).is_err()
    );
}

#[test]
fn gemini_complete_text_partition_proves_zero_images_but_omission_does_not() {
    let (mut plan, _) = configuration(Native::Gemini, true);
    drop(image_plan(&mut plan, true));
    let mut usage = fixture_value(&plan)["response"]["usageMetadata"].clone();
    usage["promptTokensDetails"] = json!([{"modality":"TEXT","tokenCount":1000}]);
    usage["cacheTokensDetails"] = json!([{"modality":"TEXT","tokenCount":200}]);
    let evidence = adapters::evidence(Native::Gemini, &usage).unwrap();
    assert_eq!(evidence.input_image, Some(0));
    assert_eq!(evidence.cached_image, Some(0));
    assert!(plan.tariff.usage(&evidence).is_ok());
    usage.as_object_mut().unwrap().remove("promptTokensDetails");
    let evidence = adapters::evidence(Native::Gemini, &usage).unwrap();
    assert_eq!(evidence.input_image, None);
    assert!(plan.tariff.usage(&evidence).is_err());
}

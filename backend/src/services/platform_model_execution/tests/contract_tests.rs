use super::*;

pub(super) fn upgrade(plan: &mut Plan, name: &str) {
    plan.model.public_id = format!("nyxid/{name}");
    for r in &mut plan.routes {
        match &mut r.target {
            RouteTarget::VertexPublisher { native_model, .. }
            | RouteTarget::DirectProvider { native_model, .. } => *native_model = name.into(),
            _ => panic!("MaaS remains exact"),
        }
    }
    let mut t = plan.tariff.tariff().clone();
    t.public_model = plan.model.public_id.clone();
    t.contract.version = t.contract.version.replace(".v1", ".v2");
    t.contract.fixture_sha256 =
        certification::digest(certification::family_fixture(t.contract.family).unwrap());
    t.source = format!(
        "contract-attested-v1:{} https://provider.example/verified-model",
        t.public_model
    );
    billing::model_tariffs::validate(&mut t, &plan.offering.slug).unwrap();
    plan.tariff = billing::model_tariffs::select(
        &plan.offering,
        &plan.model.id,
        CredentialClass::NyxidManagedMaster,
        &BTreeMap::new(),
        &[t],
        &HashMap::new(),
        Utc::now(),
        900,
    )
    .unwrap();
}
fn wire(native: Native) -> Value {
    let mut v: Value = serde_json::from_str(native.fixture()).unwrap();
    if native == Native::Responses {
        v["usage"]["input_tokens_details"]["cache_write_tokens"] = json!(0);
    }
    if native == Native::Gemini {
        v["usageMetadata"]["toolUsePromptTokenCount"] = json!(0);
    }
    v
}
async fn save(state: &crate::AppState, plan: &Plan) {
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    state
        .db
        .collection::<ModelTariff>(crate::models::platform_model_tariff::COLLECTION_NAME)
        .replace_one(doc! {"_id": &plan.tariff.tariff().id}, plan.tariff.tariff())
        .await
        .unwrap();
}

#[tokio::test]
async fn activated_model_dispatch_serves_same_name_overlap_and_rejects_unsupported_generation() {
    use crate::models::security_replica::{COLLECTION_NAME, SecurityReplica};
    use crate::services::security_readiness_service as readiness;
    let (state, mut plan, mut request) = fixture(Native::Responses, false).await;
    upgrade(&mut plan, "gpt-6-astra");
    request.body["model"] = json!(plan.model.public_id);
    save(&state, &plan).await;
    let replicas = state.db.collection::<SecurityReplica>(COLLECTION_NAME);
    let mut overlap = replicas.find_one(doc! {}).await.unwrap().unwrap();
    overlap.id = uuid::Uuid::new_v4().to_string();
    overlap.generation_id = overlap.id.clone();
    replicas.insert_one(&overlap).await.unwrap();
    let report = readiness::report(&state.db, certification::READINESS)
        .await
        .unwrap();
    assert!(report.activated && report.serving_ready && report.serving_with_roster_warnings);
    assert!(!report.ready);
    readiness::require_flag_enable(&state.db, registry::FLAG)
        .await
        .unwrap();
    registry::require_execution_ready(&state.db).await.unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_json(wire(Native::Responses)))
        .expect(1)
        .mount(&server)
        .await;
    let runtime = Runtime::mocked(&server.uri());
    let context = context(&state, &runtime, &Allowed);
    let accepted = dispatch(&context, &plan, &request).await.unwrap();
    let result = consume(&context, &plan, accepted, None).await.unwrap();
    assert_eq!(result["choices"][0]["message"]["content"], "Hello");
    let usage = state.db.collection::<UsageMeterRow>("usage_meter");
    let rows: Vec<_> = usage
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), plan.tariff.tariff().components.len());
    assert_eq!(
        rows.iter()
            .filter(|row| row.pending_platform_usage.is_some()
                || row.model_tariff.as_ref().unwrap().evidence.is_some())
            .count(),
        1
    );
    replicas
        .update_one(
            doc! {"_id": &overlap.id},
            doc! {"$pull": {"capabilities": certification::READINESS}},
        )
        .await
        .unwrap();
    assert!(matches!(
        dispatch(&context, &plan, &request).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    // Exactly one HTTP effect and no additional component charges after refusal.
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert_eq!(
        usage.count_documents(doc! {}).await.unwrap(),
        rows.len() as u64
    );
}

#[test]
fn family_contracts_accept_current_and_future_names_without_loosening_identity() {
    for (native, vertex, name) in [
        (Native::Gemini, true, "gemini-4-argon"),
        (Native::Claude, true, "claude-opus-5-5"),
        (Native::Claude, false, "claude-fable-5-2"),
        (Native::Responses, false, "gpt-6-astra"),
        (Native::Responses, false, "gpt-future-fixture"),
        (Native::Gemini, true, "gemini-future-fixture"),
        (Native::Claude, false, "claude-future-fixture"),
    ] {
        let (mut plan, request) = configuration(native, vertex);
        let old = plan.tariff.tariff().clone();
        assert!(adapters::prepare(&plan.model, &plan.routes[0], &old, &request).is_ok());
        upgrade(&mut plan, name);
        let mut request = request;
        request.body["model"] = json!(plan.model.public_id);
        assert!(
            adapters::prepare(&plan.model, &plan.routes[0], plan.tariff.tariff(), &request).is_ok()
        );
        assert!(adapters::prepare(&plan.model, &plan.routes[0], &old, &request).is_err());
        plan.model.public_id.push_str("-other");
        assert!(
            adapters::prepare(&plan.model, &plan.routes[0], plan.tariff.tariff(), &request)
                .is_err()
        );
    }
}
#[test]
fn family_contracts_require_exact_attestation_version_and_transport() {
    let (mut plan, request) = configuration(Native::Responses, false);
    upgrade(&mut plan, "gpt-6.1-sol");
    for source in [
        "",
        "https://provider.example",
        "contract-attested-v1:nyxid/gpt-other ref",
        "contract-attested-v1:nyxid/gpt-6.1-sol ",
    ] {
        let mut t = plan.tariff.tariff().clone();
        t.source = source.into();
        assert!(adapters::prepare(&plan.model, &plan.routes[0], &t, &request).is_err());
    }
    let mut t = plan.tariff.tariff().clone();
    t.contract.fixture_sha256 = Native::Responses.digest();
    assert!(billing::model_tariffs::validate(&mut t, &plan.offering.slug).is_err());
    t = plan.tariff.tariff().clone();
    t.contract.family = UsageFamily::Gemini;
    assert!(adapters::prepare(&plan.model, &plan.routes[0], &t, &request).is_err());
    let maas = RouteTarget::VertexOpenapiMaas {
        project: "private-project".into(),
        location: "global".into(),
        publisher: "openai".into(),
        native_model: "gpt-future".into(),
    };
    assert!(certification::certify(Native::Responses, &maas, plan.tariff.tariff(), false).is_err());
}
#[test]
fn family_images_reuse_limits_and_exact_or_aggregate_partitions() {
    for (native, vertex, name) in [
        (Native::Gemini, true, "gemini-3.8-flash"),
        (Native::Claude, true, "claude-sonnet-5-5"),
        (Native::Claude, false, "claude-haiku-5-5"),
        (Native::Responses, false, "gpt-6-luna"),
    ] {
        let (mut plan, _) = configuration(native, vertex);
        let mut request = images_tests::image_plan(&mut plan, native == Native::Gemini);
        upgrade(&mut plan, name);
        request.body["model"] = json!(plan.model.public_id);
        assert!(
            adapters::prepare(&plan.model, &plan.routes[0], plan.tariff.tariff(), &request).is_ok()
        );
        assert!(plan.image_limits().is_some());
        let mut t = plan.tariff.tariff().clone();
        t.bounds.max_images = 0;
        assert!(adapters::prepare(&plan.model, &plan.routes[0], &t, &request).is_err());
    }
}
#[test]
fn strict_contract_usage_rejects_unknown_fields_nested_classes_and_non_token_costs() {
    assert!(contract_usage::envelope(Native::Responses, &json!({"tool_usage":[]})).is_err());
    for native in [Native::Responses, Native::Claude, Native::Gemini] {
        let f: Value =
            serde_json::from_str(certification::family_fixture(native.family()).unwrap()).unwrap();
        let mut usage = f["usage"].clone();
        assert!(contract_usage::validate(native, &usage).is_ok());
        usage["future_charge"] = json!(0);
        assert!(contract_usage::validate(native, &usage).is_err());
    }
    for (native, path, value) in [
        (
            Native::Responses,
            "/input_tokens_details",
            json!({"cached_tokens":0,"audio_tokens":0}),
        ),
        (
            Native::Responses,
            "/output_tokens_details",
            json!({"reasoning_tokens":0,"new_tokens":0}),
        ),
        (
            Native::Responses,
            "/input_tokens_details",
            json!({"cached_tokens":0,"cache_write_tokens_details":{"ttl_1h_tokens":0}}),
        ),
        (
            Native::Claude,
            "/cache_creation",
            json!({"ephemeral_1h_input_tokens":0,"new_tokens":0}),
        ),
        (
            Native::Claude,
            "/output_tokens_details",
            json!({"thinking_tokens":-1}),
        ),
        (
            Native::Claude,
            "/server_tool_use",
            json!({"web_search_requests":1}),
        ),
        (Native::Claude, "/speed", json!("fast")),
        (
            Native::Gemini,
            "/promptTokensDetails",
            json!([{"modality":"AUDIO","tokenCount":0}]),
        ),
        (
            Native::Gemini,
            "/promptTokensDetails",
            json!([{"modality":"TEXT","tokenCount":1000,"new":0}]),
        ),
    ] {
        let f: Value =
            serde_json::from_str(certification::family_fixture(native.family()).unwrap()).unwrap();
        let mut usage = f["usage"].clone();
        let key = path.trim_start_matches('/');
        usage[key] = value;
        assert!(
            contract_usage::validate(native, &usage).is_err(),
            "{native:?} {path}"
        );
    }
}
#[test]
fn modern_cache_writes_reasoning_and_tool_inputs_are_never_free_or_guessed() {
    for native in [Native::Responses, Native::Claude, Native::Gemini] {
        let (mut plan, _) = configuration(native, native == Native::Gemini);
        let name = plan.model.public_id.trim_start_matches("nyxid/").to_owned();
        upgrade(&mut plan, &name);
        let mut c = plan.tariff.tariff().contract.clone();
        if native == Native::Responses {
            c.cache_write_ttls = vec![CacheTtl::ThirtyMinutes, CacheTtl::TwelveHours];
        }
        let f: Value =
            serde_json::from_str(certification::family_fixture(native.family()).unwrap()).unwrap();
        let e = contract_usage::evidence(native, &f["usage"], &c).unwrap();
        let n = billing::model_usage::normalize(&c, &e).unwrap();
        match native {
            Native::Responses => {
                assert_eq!(n[&Selector::Input], 1000);
                assert_eq!(
                    n[&Selector::CacheWrite {
                        ttl: CacheTtl::ThirtyMinutes
                    }],
                    3000
                );
                assert_eq!(
                    n[&Selector::CacheWrite {
                        ttl: CacheTtl::TwelveHours
                    }],
                    4000
                );
            }
            Native::Claude => assert_eq!(n[&Selector::Output], 100),
            Native::Gemini => assert_eq!(
                n[&Selector::Output] + n.get(&Selector::Reasoning).copied().unwrap_or(0),
                150
            ),
            _ => unreachable!(),
        }
        let mut missing = f["usage"].clone();
        missing
            .as_object_mut()
            .unwrap()
            .remove(if native == Native::Gemini {
                "thoughtsTokenCount"
            } else {
                "output_tokens"
            });
        assert!(contract_usage::evidence(native, &missing, &c).is_err());
        if native == Native::Gemini {
            let mut u = f["usage"].clone();
            u["toolUsePromptTokenCount"] = json!(10);
            u["totalTokenCount"] = json!(1160);
            assert!(contract_usage::evidence(native, &u, &c).is_err());
            c.bill_tool_input = true;
            assert_eq!(
                billing::model_usage::normalize(
                    &c,
                    &contract_usage::evidence(native, &u, &c).unwrap()
                )
                .unwrap()[&Selector::Input],
                810
            );
        }
    }
}
#[test]
fn unknown_intermediate_usage_cannot_be_hidden_by_a_valid_final_snapshot() {
    let (mut plan, _) = configuration(Native::Responses, false);
    upgrade(&mut plan, "gpt-6-astra");
    let mut decoder =
        stream::Decoder::new(Native::Responses).with_contract(&plan.tariff.tariff().contract);
    let bad = json!({"type":"response.in_progress","response":{"usage":{"future_tokens":0}}});
    assert!(decoder.push(format!("data: {bad}\n\n").as_bytes()).is_err());
    let good = json!({"type":"response.completed","response":wire(Native::Responses)});
    decoder
        .push(format!("data: {good}\n\n").as_bytes())
        .unwrap();
    assert!(decoder.finish(true, "public").is_err());
    assert!(decoder.partial_evidence().is_none());
}
#[tokio::test]
async fn new_contracts_require_fleet_readiness_before_credentials_or_funding() {
    let (state, mut plan, request) = fixture(Native::Responses, false).await;
    upgrade(&mut plan, "gpt-6-astra");
    let heartbeat =
        crate::services::security_readiness_service::controlled_ready_test_replica(&state.db).await;
    heartbeat.abort();
    let _ = heartbeat.await;
    state
        .db
        .collection::<Document>(crate::models::security_replica::COLLECTION_NAME)
        .update_many(
            doc! {},
            doc! {"$pull":{"capabilities":certification::READINESS}},
        )
        .await
        .unwrap();
    assert!(
        crate::services::security_readiness_service::report(
            &state.db,
            registry::EXECUTION_READINESS
        )
        .await
        .unwrap()
        .ready
    );
    state
        .db
        .collection::<Document>("downstream_services")
        .delete_one(doc! {"_id":&plan.routes[0].credential_service_id})
        .await
        .unwrap();
    let server = MockServer::start().await;
    let runtime = Runtime::mocked(&server.uri());
    let ctx = context(&state, &runtime, &Allowed);
    assert!(matches!(
        dispatch(&ctx, &plan, &request).await,
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
async fn current_family_models_execute_through_the_existing_meter_and_opaque_response() {
    for (native, vertex, name) in [
        (Native::Gemini, true, "gemini-4-argon"),
        (Native::Claude, true, "claude-opus-5-5"),
        (Native::Claude, false, "claude-fable-5-2"),
        (Native::Responses, false, "gpt-6.1-sol"),
    ] {
        let (state, mut plan, mut request) = fixture(native, vertex).await;
        upgrade(&mut plan, name);
        request.body["model"] = json!(plan.model.public_id);
        save(&state, &plan).await;
        let server = MockServer::start().await;
        token_mock(&server).await;
        Mock::given(method("POST"))
            .and(path_regex("^/v1/.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(wire(native)))
            .expect(1)
            .mount(&server)
            .await;
        let runtime = Runtime::mocked(&server.uri());
        let ctx = context(&state, &runtime, &Allowed);
        let accepted = dispatch(&ctx, &plan, &request).await.unwrap();
        let result = consume(&ctx, &plan, accepted, None).await.unwrap();
        assert_eq!(result["model"], plan.model.public_id);
        assert!(!result.to_string().contains("private"));
        let rows: Vec<UsageMeterRow> = state
            .db
            .collection("usage_meter")
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(rows.len(), plan.tariff.tariff().components.len());
        assert!(rows.iter().all(|r| r.quantity.is_some()));
        server.verify().await;
    }
}
#[tokio::test]
async fn unsupported_usage_preserves_holds_without_fallback_or_recovery_redispatch() {
    for bad in [
        json!({"future_tokens":0}),
        json!({"input_tokens":100,"output_tokens":20,"total_tokens":120,"input_tokens_details":{"cached_tokens":10,"cache_write_tokens":0,"audio_tokens":1},"output_tokens_details":{"reasoning_tokens":5}}),
        json!({"input_tokens":100,"output_tokens":20,"total_tokens":120,"input_tokens_details":{"cached_tokens":10,"cache_write_tokens":1},"output_tokens_details":{"reasoning_tokens":5}}),
        Value::Null,
    ] {
        let (state, mut plan, request) = fixture(Native::Responses, false).await;
        upgrade(&mut plan, "gpt-6-astra");
        save(&state, &plan).await;
        let mut backup = plan.routes[0].clone();
        backup.id = uuid::Uuid::new_v4().to_string();
        plan.routes.push(backup);
        let mut body = wire(Native::Responses);
        body["usage"] = bad;
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
        let runtime = Runtime::mocked(&server.uri());
        let ctx = context(&state, &runtime, &Allowed);
        let accepted = dispatch(&ctx, &plan, &request).await.unwrap();
        assert!(
            matches!(consume(&ctx,&plan,accepted,None).await, Err(AppError::BillingProviderUnavailable(reason)) if reason == "model_usage_pending")
        );
        recover(&state.db).await.unwrap();
        recover(&state.db).await.unwrap();
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
        assert!(rows.iter().all(|r| r.forwarded
            && !r.released
            && r.quantity.is_none()
            && r.last_error.as_deref() == Some("model_usage_pending")));
        server.verify().await;
    }
}

#[test]
fn modern_thinking_is_aggregate_private_and_signed_tool_continuations_are_refused() {
    for native in [Native::Claude, Native::Gemini] {
        let (mut plan, _) = configuration(native, native == Native::Gemini);
        let name = plan.model.public_id.trim_start_matches("nyxid/").to_owned();
        upgrade(&mut plan, &name);
        let mut body = wire(native);
        if native == Native::Claude {
            body["content"].as_array_mut().unwrap().insert(0,json!({"type":"thinking","thinking":"private reasoning","signature":"private signature"}));
        } else {
            body["candidates"][0]["content"]["parts"][0]["thoughtSignature"] =
                json!("private signature");
        }
        let out =
            adapters::output_contract(native, &body, "public", &plan.tariff.tariff().contract)
                .unwrap();
        assert_eq!(out.text, "Hello");
        if native == Native::Claude {
            body["content"]
                .as_array_mut()
                .unwrap()
                .push(json!({"type":"tool_use","name":"tool","input":{}}));
        } else {
            body["candidates"][0]["content"]["parts"][0] = json!({"functionCall":{"name":"tool","args":{}},"thoughtSignature":"private signature"});
        }
        assert!(
            adapters::output_contract(native, &body, "public", &plan.tariff.tariff().contract)
                .is_err()
        );
    }
}
#[test]
fn gemini_contract_requires_usage_at_or_after_terminal_candidate_not_an_earlier_snapshot() {
    let (mut plan, _) = configuration(Native::Gemini, true);
    upgrade(&mut plan, "gemini-4-argon");
    let mut decoder =
        stream::Decoder::new(Native::Gemini).with_contract(&plan.tariff.tariff().contract);
    let mut first = wire(Native::Gemini);
    first["candidates"][0]
        .as_object_mut()
        .unwrap()
        .remove("finishReason");
    decoder
        .push(format!("data: {first}\n\n").as_bytes())
        .unwrap();
    let mut last = wire(Native::Gemini);
    last.as_object_mut().unwrap().remove("usageMetadata");
    decoder
        .push(format!("data: {last}\n\n").as_bytes())
        .unwrap();
    assert!(decoder.finish(true, "public").is_err());
}
#[test]
fn current_contract_terminal_streams_accept_complete_evidence() {
    for native in [Native::Responses, Native::Claude, Native::Gemini] {
        let (mut plan, _) = configuration(native, native == Native::Gemini);
        let name = plan.model.public_id.trim_start_matches("nyxid/").to_owned();
        upgrade(&mut plan, &name);
        let mut decoder =
            stream::Decoder::new(native).with_contract(&plan.tariff.tariff().contract);
        let wire = wire(native);
        let events = match native {
            Native::Responses => vec![json!({"type":"response.completed","response":wire})],
            Native::Claude => vec![
                json!({"type":"message_start","message":{"usage":wire["usage"]}}),
                json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
                json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"private"}}),
                json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"private"}}),
                json!({"type":"content_block_stop","index":0}),
                json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":"Hello"}}),
                json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":wire["usage"]["output_tokens"],"output_tokens_details":{"thinking_tokens":5}}}),
                json!({"type":"message_stop"}),
            ],
            Native::Gemini => vec![wire],
            _ => unreachable!(),
        };
        let mut texts = String::new();
        for e in events {
            texts.push_str(
                &decoder
                    .push(format!("data: {e}\n\n").as_bytes())
                    .unwrap()
                    .join(""),
            );
        }
        assert!(!texts.contains("private"));
        let (out, evidence) = decoder.finish(true, "public").unwrap();
        assert_eq!(out.unwrap().text, "Hello");
        assert!(plan.tariff.usage(&evidence).is_ok());
    }
}

#[test]
fn family_image_bundles_have_complete_nonstream_and_stream_accounting() {
    for (native, vertex, name) in [
        (Native::Gemini, true, "gemini-4-argon"),
        (Native::Claude, true, "claude-opus-5-5"),
        (Native::Claude, false, "claude-haiku-5-5"),
        (Native::Responses, false, "gpt-6-luna"),
    ] {
        let (mut plan, _) = configuration(native, vertex);
        drop(images_tests::image_plan(
            &mut plan,
            native == Native::Gemini,
        ));
        upgrade(&mut plan, name);
        let bundle: Value =
            serde_json::from_str(certification::family_fixture(native.family()).unwrap()).unwrap();
        let key = if native == Native::Claude && vertex {
            "vertex_image_fixture"
        } else {
            "image_fixture"
        };
        let fixture = &bundle[key];
        let mut body = fixture["request"].clone();
        body["model"] = json!(plan.model.public_id);
        let request = Request::parse(&http::Method::POST, "chat/completions", body).unwrap();
        assert!(
            adapters::prepare(&plan.model, &plan.routes[0], plan.tariff.tariff(), &request).is_ok()
        );
        let c = &plan.tariff.tariff().contract;
        let usage_key = if native == Native::Gemini {
            "usageMetadata"
        } else {
            "usage"
        };
        let evidence =
            contract_usage::evidence(native, &fixture["response"][usage_key], c).unwrap();
        let expected = plan.tariff.usage(&evidence).unwrap();
        let mut decoder = stream::Decoder::new(native).with_contract(c);
        for event in fixture["stream_events"].as_array().unwrap() {
            decoder
                .push(format!("data: {event}\n\n").as_bytes())
                .unwrap();
        }
        let (output, evidence) = decoder.finish(true, "public").unwrap();
        assert_eq!(output.unwrap().text, "Hello");
        assert_eq!(plan.tariff.usage(&evidence).unwrap(), expected);
    }
}

#[test]
fn zero_write_aggregate_proves_zero_partitions_but_missing_or_positive_does_not() {
    let (mut plan, _) = configuration(Native::Responses, false);
    upgrade(&mut plan, "gpt-6-astra");
    let mut c = plan.tariff.tariff().contract.clone();
    c.cache_write_ttls = vec![CacheTtl::ThirtyMinutes, CacheTtl::TwelveHours];
    let mut body = wire(Native::Responses);
    let evidence = contract_usage::evidence(Native::Responses, &body["usage"], &c).unwrap();
    let normalized = billing::model_usage::normalize(&c, &evidence).unwrap();
    assert_eq!(
        normalized[&Selector::CacheWrite {
            ttl: CacheTtl::ThirtyMinutes
        }],
        0
    );
    assert_eq!(
        normalized[&Selector::CacheWrite {
            ttl: CacheTtl::TwelveHours
        }],
        0
    );
    body["usage"]["input_tokens_details"]["cache_write_tokens"] = json!(1);
    let evidence = contract_usage::evidence(Native::Responses, &body["usage"], &c).unwrap();
    assert!(billing::model_usage::normalize(&c, &evidence).is_err());
    body["usage"]["input_tokens_details"]
        .as_object_mut()
        .unwrap()
        .remove("cache_write_tokens");
    assert!(contract_usage::evidence(Native::Responses, &body["usage"], &c).is_err());
}

#[test]
fn nonterminal_provider_responses_never_supply_final_usage() {
    for native in [Native::Responses, Native::Claude, Native::Gemini] {
        let mut body = wire(native);
        assert!(contract_usage::final_response(native, &body).is_ok());
        match native {
            Native::Responses => body["status"] = json!("in_progress"),
            Native::Claude => body["stop_reason"] = Value::Null,
            Native::Gemini => {
                body["candidates"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("finishReason");
            }
            _ => unreachable!(),
        }
        assert!(contract_usage::final_response(native, &body).is_err());
    }
}

#[test]
fn duplicate_wire_keys_cannot_erase_usage_including_nested_or_escaped_keys() {
    for body in [
        r#"{"usage":{"future_tokens":1},"usage":{"input_tokens":0}}"#,
        r#"{"usage":{"input_tokens_details":{"cached_tokens":1,"cached_tokens":0}}}"#,
        r#"{"usage":{"future_tokens":1},"us\u0061ge":{}}"#,
        r#"{"error":{"usage":{"future_tokens":1}},"error":{"code":"rate_limit_exceeded"}}"#,
    ] {
        assert!(contract_usage::parse(body.as_bytes()).is_err());
    }
    let body = json!({"array":[null,true,1,2.5,"text",{"key":"value"}]});
    assert_eq!(
        contract_usage::parse(body.to_string().as_bytes()).unwrap(),
        body
    );
    let (mut plan, _) = configuration(Native::Responses, false);
    upgrade(&mut plan, "gpt-6-astra");
    let mut decoder =
        stream::Decoder::new(Native::Responses).with_contract(&plan.tariff.tariff().contract);
    let completed = json!({"type":"response.completed","response":wire(Native::Responses)});
    decoder
        .push(format!("data: {completed}\n\n").as_bytes())
        .unwrap();
    assert!(
        decoder
            .push(b"data: {\"usage\":{\"future_tokens\":1},\"usage\":{}}\n\n")
            .is_err()
    );
    assert!(decoder.finish(true, "public").is_err());
    assert!(decoder.partial_evidence().is_none());
}

#[tokio::test]
async fn ambiguous_json_holds_usage_and_never_authorizes_fallback() {
    for (status, body) in [
        (200, r#"{"status":"completed","usage":{"future_tokens":1},"usage":{"input_tokens":100,"output_tokens":20,"total_tokens":120,"input_tokens_details":{"cached_tokens":10,"cache_write_tokens":0},"output_tokens_details":{"reasoning_tokens":5}}}"#.to_owned()),
        (429, r#"{"error":{"usage":{"future_tokens":1}},"error":{"code":"rate_limit_exceeded"}}"#.to_owned()),
    ] {
        let (state, mut plan, request) = fixture(Native::Responses, false).await;
        upgrade(&mut plan, "gpt-6-astra"); save(&state, &plan).await;
        let mut backup = plan.routes[0].clone(); backup.id = uuid::Uuid::new_v4().to_string(); plan.routes.push(backup);
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(status).set_body_raw(body,"application/json"))
            .expect(1).mount(&server).await;
        let runtime = Runtime::mocked(&server.uri()); let ctx = context(&state, &runtime, &Allowed);
        let result = match dispatch(&ctx, &plan, &request).await {
            Ok(accepted) => consume(&ctx, &plan, accepted, None).await,
            Err(error) => Err(error),
        };
        assert!(matches!(result, Err(AppError::BillingProviderUnavailable(_))));
        let rows: Vec<UsageMeterRow> = state.db.collection("usage_meter").find(doc!{}).await.unwrap().try_collect().await.unwrap();
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|r| r.forwarded && !r.released && r.quantity.is_none() && r.last_error.as_deref() == Some("model_usage_pending")));
        server.verify().await;
    }
}

#[test]
fn contradictory_or_incomplete_stream_tails_cannot_reuse_earlier_terminal_counts() {
    let (mut plan, _) = configuration(Native::Responses, false);
    upgrade(&mut plan, "gpt-6-astra");
    for tail in [
        b"data: {\"type\":\"response.completed\",\"sequence_number\":2,\"response\":{}}\n\n"
            .as_slice(),
        b"data: {\"type\":\"response.completed\",\"sequence_number\":0,\"response\":{}}\n\n"
            .as_slice(),
        b"data: {\"type\":\"response.completed\",\"sequence_number\":-1}\n\n".as_slice(),
        b"data: {\"usage\":".as_slice(),
        b"data: [DONE]\n\n".as_slice(),
        b"data: \xff\n\n".as_slice(),
    ] {
        let mut decoder =
            stream::Decoder::new(Native::Responses).with_contract(&plan.tariff.tariff().contract);
        let completed = json!({"type":"response.completed","sequence_number":1,"response":wire(Native::Responses)});
        decoder
            .push(format!("data: {completed}\n\n").as_bytes())
            .unwrap();
        let _ = decoder.push(tail);
        assert!(decoder.finish(true, "public").is_err());
        assert!(decoder.partial_evidence().is_none());
    }
    let (mut plan, _) = configuration(Native::Gemini, true);
    upgrade(&mut plan, "gemini-4-argon");
    let mut decoder =
        stream::Decoder::new(Native::Gemini).with_contract(&plan.tariff.tariff().contract);
    decoder
        .push(format!("data: {}\n\n", wire(Native::Gemini)).as_bytes())
        .unwrap();
    let more = json!({"candidates":[{"content":{"parts":[{"text":"unexpected continuation"}]}}]});
    assert!(
        decoder
            .push(format!("data: {more}\n\n").as_bytes())
            .is_err()
    );
    assert!(decoder.finish(true, "public").is_err());
    assert!(decoder.partial_evidence().is_none());
}

#[test]
fn gemini_usage_only_final_frame_accepts_empty_candidates_with_exact_evidence() {
    let (mut plan, _) = configuration(Native::Gemini, true);
    upgrade(&mut plan, "gemini-4-argon");
    let mut decoder =
        stream::Decoder::new(Native::Gemini).with_contract(&plan.tariff.tariff().contract);
    let mut last_candidate = wire(Native::Gemini);
    let usage = last_candidate
        .as_object_mut()
        .unwrap()
        .remove("usageMetadata")
        .unwrap();
    decoder
        .push(format!("data: {last_candidate}\n\n").as_bytes())
        .unwrap();
    let final_usage = json!({"candidates":[],"usageMetadata":usage});
    decoder
        .push(format!("data: {final_usage}\n\n").as_bytes())
        .unwrap();
    let (output, evidence) = decoder.finish(true, "public").unwrap();
    assert_eq!(output.unwrap().text, "Hello");
    assert!(plan.tariff.usage(&evidence).is_ok());
}

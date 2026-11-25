const MARKERS: &str = "assistant_migrations";
const MARKER: &str = "cma-legacy-sandboxes-v1";

async fn legacy_fixture() -> Fixture {
    let f = Fixture::new().await;
    f.db.collection::<bson::Document>(MARKERS).insert_one(doc! {
        "_id": MARKER, "cutoff": bson::DateTime::parse_rfc3339_str("2026-01-02T00:00:00Z").unwrap(),
        "rule": "earliest_creation"
    }).await.unwrap();
    Mock::given(method("GET")).and(path("/api/v1/sandboxes/sandbox_1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id":"sandbox_1", "owner":f.owner, "status":"active", "created_at":"2026-01-01T00:00:00Z"
        }))).mount(&f.server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/agents/agent_1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id":"agent_1", "owner":f.owner, "sandbox_id":"sandbox_1", "status":"ready"
        })))
        .mount(&f.server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/workspaces/workspace_1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"id":"workspace_1","sandbox_id":"sandbox_1"})),
        )
        .mount(&f.server)
        .await;
    f
}

async fn permit_for(
    f: &Fixture,
    verb: &str,
    route: &str,
    body: &[u8],
    headers: &[(String, String)],
    query: Option<&str>,
) -> AppResult<CmaDispatchPermit> {
    admit(
        &f.db, &f.keys, &f.jwt, &f.config, &f.owner, true, &f.target, verb, route, query, body,
        headers, false,
    )
    .await
}

async fn assert_no_managed_rows(f: &Fixture) {
    for collection in [
        "assistant_cma_sandboxes",
        "assistant_cma_agents",
        "assistant_cma_creations",
        "sandbox_billing_quotes",
        "sandbox_billing_windows",
    ] {
        assert_eq!(
            f.db.collection::<bson::Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0,
            "{collection}"
        );
    }
}

#[tokio::test]
async fn legacy_safe_operations_preserve_wire_and_response_without_managed_rows() {
    let f = legacy_fixture().await;
    let key = vec![("Idempotency-Key".into(), "legacy-key".into())];
    let control =
        br#"{ "turn_sequence":1,"expected_agent_revision":1,"command":{"type":"cancel"}}"#;
    for (verb, route, body) in [
        (
            "POST",
            "/api/v1/sandboxes/sandbox_1/pause",
            &b"{ \"opaque\": true }"[..],
        ),
        ("DELETE", "/api/v1/sandboxes/sandbox_1", &b""[..]),
        ("POST", "/api/v1/agents/agent_1/stop", &b"{}"[..]),
        ("DELETE", "/api/v1/agents/agent_1", &b""[..]),
        (
            "POST",
            "/api/v1/agents/agent_1/files",
            &b"{ \"path\": \"example.txt\", \"content_base64\": \"\" }"[..],
        ),
        (
            "POST",
            "/api/v2/agents/agent_1/responses/response_1/controls",
            &control[..],
        ),
    ] {
        Mock::given(method(verb))
            .and(path(route))
            .respond_with(
                ResponseTemplate::new(409)
                    .insert_header("x-cma-test", "unchanged")
                    .set_body_raw("{ \"upstream\": \"refusal\" }", "application/problem+json"),
            )
            .mount(&f.server)
            .await;
        let permit = permit_for(&f, verb, route, body, &key, None)
            .await
            .expect("verified legacy parent passes through");
        assert!(permit.creation.is_none() && permit.sandbox.is_none());
        let request = reqwest::Client::new()
            .request(verb.parse().unwrap(), format!("{}{route}", f.server.uri()))
            .header("idempotency-key", "legacy-key")
            .header("x-client-test", "preserved")
            .body(body.to_vec())
            .build()
            .unwrap();
        permit.before_send(&request).await.unwrap();
        let response = permit
            .observe_response(reqwest::Client::new().execute(request).await.unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), 409);
        assert_eq!(response.headers()["x-cma-test"], "unchanged");
        assert_eq!(
            response.bytes().await.unwrap(),
            "{ \"upstream\": \"refusal\" }"
        );
        let requests = f.server.received_requests().await.unwrap();
        let forwarded = requests.last().unwrap();
        assert_eq!(forwarded.body, body);
        assert_eq!(forwarded.headers["x-client-test"], "preserved");
        assert_eq!(forwarded.headers["idempotency-key"], "legacy-key");
    }
    assert_no_managed_rows(&f).await;
    assert!(matches!(
        sandboxes::resolve_created(&f.db, &f.gateway(), &f.owner, "sandbox_1").await,
        Err(AppError::NotFound(_))
    ));
    let created = f
        .admit(
            &f.target,
            "/api/v1/sandboxes",
            "new-tracked-key",
            br#"{"machine_included":false}"#,
        )
        .await
        .unwrap();
    assert!(created.creation.is_some());
    f.assert_single_intent().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_keyless_query_requests_keep_the_original_bytes() {
    let f = legacy_fixture().await;
    let body = b"{ \"reason\": \"example\" }";
    let permit = permit_for(
        &f,
        "POST",
        "/api/v1/sandboxes/sandbox_1/pause",
        body,
        &[],
        Some("example=1"),
    )
    .await
    .expect("legacy needs no managed key");
    let request = reqwest::Client::new()
        .post(format!(
            "{}/api/v1/sandboxes/sandbox_1/pause?example=1",
            f.server.uri()
        ))
        .body(body.to_vec())
        .build()
        .unwrap();
    permit.before_send(&request).await.unwrap();
    assert_no_managed_rows(&f).await;
    f.db.drop().await.unwrap();
}

async fn monitored(f: &mut Fixture) -> std::sync::Arc<std::sync::Mutex<Vec<bson::Document>>> {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let commands = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = commands.clone();
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            recorded.lock().unwrap().push(event.command);
        }
    }));
    f.db = mongodb::Client::with_options(options)
        .unwrap()
        .database(f.db.name());
    commands
}

async fn tracked_parent(f: &Fixture) {
    f.db.collection::<bson::Document>(crate::models::assistant_cma_sandbox::COLLECTION_NAME).insert_one(doc! {
        "_id":"00000000-0000-4000-8000-000000000001", "user_id":&f.owner, "cma_service_id":&f.catalog.id,
        "cma_sandbox_id":"sandbox_1", "label":"Sandbox", "source":"nyxbot", "creation_ref":"example",
        "cpu_millis":2000, "memory_mib":2048, "capacity_source":"profile_revision",
        "generation":1_i64, "occupies_account_slot":true, "admission_state":"associated", "observation_state":"ready",
        "created_at":bson::DateTime::parse_rfc3339_str("2025-12-01T00:00:00Z").unwrap(), "updated_at":bson::DateTime::now()
    }).await.unwrap();
}

fn normalized_commands(commands: &[bson::Document], f: &Fixture) -> String {
    // Arrays of field pairs preserve BSON document order; canonical extended
    // JSON preserves BSON types (including Int32 versus Int64).
    fn ordered(value: &bson::Bson) -> Value {
        match value {
            bson::Bson::Document(document) => serde_json::json!({"$document": document.iter()
                .filter(|(key,_)| !matches!(key.as_str(),"lsid"|"$clusterTime"|"$db"|"txnNumber"|"operationTime"))
                .map(|(key,value)| {
                    let value = if key=="request_ciphertext" && let bson::Bson::Binary(binary)=value {
                        let mut binary=binary.clone();
                        binary.bytes.fill(0);
                        bson::Bson::Binary(binary).into_canonical_extjson()
                    } else { ordered(value) };
                    serde_json::json!([key,value])
                }).collect::<Vec<_>>() }),
            bson::Bson::Array(values) => Value::Array(values.iter().map(ordered).collect()),
            bson::Bson::DateTime(_) => serde_json::json!({"$date":"<time>"}),
            other => other.clone().into_canonical_extjson(),
        }
    }
    let value = Value::Array(
        commands
            .iter()
            .map(|command| ordered(&bson::Bson::Document(command.clone())))
            .collect(),
    );
    serde_json::to_string_pretty(&value)
        .unwrap()
        .replace(&f.owner, "<owner>")
        .replace(&f.catalog.id, "<catalog>")
        .replace(&catalog_execution_fingerprint(&f.catalog), "<deployment>")
}

#[tokio::test]
async fn tracked_commands_and_wire_match_main() {
    let mut traces = serde_json::Map::new();
    for (verb, route, body) in [
        (
            "POST",
            "/api/v1/sandboxes/sandbox_1/pause",
            r#"{ "example": true }"#,
        ),
        ("DELETE", "/api/v1/sandboxes/sandbox_1", ""),
        ("POST", "/api/v1/agents/agent_1/stop", "{}"),
        ("DELETE", "/api/v1/agents/agent_1", ""),
        (
            "POST",
            "/api/v1/agents/agent_1/files",
            r#"{"path":"example.txt","content_base64":""}"#,
        ),
        (
            "POST",
            "/api/v2/agents/agent_1/responses/response_1/controls",
            r#"{"turn_sequence":1,"expected_agent_revision":1,"command":{"type":"cancel"}}"#,
        ),
        (
            "POST",
            "/api/v1/agents",
            r#"{ "name": "example", "sandbox_id": "sandbox_1" }"#,
        ),
        (
            "POST",
            "/api/v1/agents",
            r#"{ "name": "example", "workspace_id": "workspace_1" }"#,
        ),
        (
            "POST",
            "/api/v1/workspaces/workspace_1/agents",
            r#"{ "name": "example" }"#,
        ),
        ("POST", "/api/v1/sandboxes/sandbox_1/revive", "{}"),
        ("POST", "/api/v1/sandboxes/sandbox_1/workspaces", "{}"),
        (
            "POST",
            "/api/v1/sandboxes/sandbox_1/workspaces/inline",
            "{}",
        ),
        ("POST", "/api/v1/agents/agent_1/input", "{}"),
        (
            "POST",
            "/api/v1/responses",
            r#"{"conversation":"agent_1","stream":true}"#,
        ),
        (
            "POST",
            "/api/v2/responses",
            r#"{"conversation":"agent_1","stream":true}"#,
        ),
        (
            "POST",
            "/api/v2/agents/agent_1/responses",
            r#"{"stream":true}"#,
        ),
        (
            "POST",
            "/api/v2/agents/agent_1/responses/response_1/interrupt",
            r#"{"turn_sequence":1,"expected_agent_revision":1,"text":"example"}"#,
        ),
    ] {
        let mut f = legacy_fixture().await;
        tracked_parent(&f).await;
        Mock::given(method(verb))
            .and(path(route))
            .respond_with(
                ResponseTemplate::new(409)
                    .insert_header("x-cma-test", "unchanged")
                    .set_body_raw("{ \"upstream\": \"refusal\" }", "application/problem+json"),
            )
            .mount(&f.server)
            .await;
        let recorded = monitored(&mut f).await;
        let headers = vec![("Idempotency-Key".into(), "tracked-key".into())];
        let permit = permit_for(&f, verb, route, body.as_bytes(), &headers, None)
            .await
            .unwrap();
        let request = reqwest::Client::new()
            .request(verb.parse().unwrap(), format!("{}{route}", f.server.uri()))
            .header("idempotency-key", "tracked-key")
            .body(body.to_owned())
            .build()
            .unwrap();
        permit.before_send(&request).await.unwrap();
        assert_eq!(request.body().unwrap().as_bytes().unwrap(), body.as_bytes());
        assert_eq!(request.headers()["idempotency-key"], "tracked-key");
        let response = permit
            .observe_response(reqwest::Client::new().execute(request).await.unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), 409);
        assert_eq!(response.headers()["x-cma-test"], "unchanged");
        assert_eq!(
            response.bytes().await.unwrap(),
            "{ \"upstream\": \"refusal\" }"
        );
        let requests = f.server.received_requests().await.unwrap();
        assert_eq!(requests.last().unwrap().body, body.as_bytes());
        assert_eq!(
            requests.last().unwrap().headers["idempotency-key"],
            "tracked-key"
        );
        let commands = recorded.lock().unwrap().clone();
        let live_updates: Vec<_> = commands
            .iter()
            .filter(|cmd| {
                cmd.get_str("findAndModify")
                    == Ok(crate::models::assistant_cma_sandbox::COLLECTION_NAME)
            })
            .filter_map(|cmd| cmd.get_document("update").ok()?.get_document("$set").ok())
            .filter(|set| set.get_str("observation_state") == Ok("ready"))
            .collect();
        let [set] = live_updates.as_slice() else {
            panic!("{verb} {route} must persist exactly one live sandbox observation");
        };
        for field in ["request_ciphertext", "request_context_version"] {
            assert!(!set.contains_key(field), "{verb} {route} wrote {field}");
        }
        let mut normalized = normalized_commands(&commands, &f);
        if let Some(creation) = &permit.creation {
            normalized = normalized.replace(&creation.id, "<creation>");
        }
        assert!(!normalized.contains(MARKER));
        let wire: Vec<_>=requests.iter().map(|request| {
            let headers:std::collections::BTreeMap<_,_>=request.headers.iter()
                .filter(|(name,_)| !matches!(name.as_str(),"host"|"x-nyxid-identity-token"))
                .map(|(name,value)|(name.as_str(),value.to_str().unwrap())).collect();
            serde_json::json!({"method":request.method.as_str(),"path":request.url.path(),"query":request.url.query(),"headers":headers,"body":request.body})
        }).collect();
        traces.insert(format!("{verb} {route} {body}"),serde_json::json!({"commands":serde_json::from_str::<Value>(&normalized).unwrap(),"wire":wire}));

        f.db.drop().await.unwrap();
    }
    // Captured on origin/main 4a31e80cae8eb2aea8f1cae327bc3321551a54f2.
    // Driver 3.7 places top-level writeConcern after command metadata. Moving
    // that field back reproduced the original fingerprint for all 17 traces.
    // Normalize only session/time/fixture identities, ciphertext and signed JWTs.
    // These pins deliberately differ from main because a live observation no longer writes the request pair (#1956 PR 9).
    let expected: std::collections::BTreeMap<String, String> =
        serde_json::from_str(include_str!("assistant_cma_legacy_tracked_commands.json")).unwrap();
    assert_eq!(traces.len(), expected.len());
    for (operation, trace) in &traces {
        let actual = hex::encode(Sha256::digest(serde_json::to_vec(trace).unwrap()));
        assert_eq!(
            Some(&actual),
            expected.get(operation),
            "tracked commands or wire changed: {operation}\n{trace:#}"
        );
    }
}

#[tokio::test]
async fn legacy_missing_boundary_bad_future_and_foreign_proofs_stay_closed() {
    let f = legacy_fixture().await;
    for (owner, timestamp, id) in [
        (
            Some(f.owner.as_str()),
            Some("2026-01-02T00:00:00Z"),
            "sandbox_1",
        ),
        (
            Some(f.owner.as_str()),
            Some("2026-01-03T00:00:00Z"),
            "sandbox_1",
        ),
        (
            Some(f.owner.as_str()),
            Some("2999-01-01T00:00:00Z"),
            "sandbox_1",
        ),
        (Some(f.owner.as_str()), Some("bad"), "sandbox_1"),
        (Some(f.owner.as_str()), None, "sandbox_1"),
        (
            Some("foreign-owner"),
            Some("2026-01-01T00:00:00Z"),
            "sandbox_1",
        ),
        (None, Some("2026-01-01T00:00:00Z"), "sandbox_1"),
        (
            Some(f.owner.as_str()),
            Some("2026-01-01T00:00:00Z"),
            "different_sandbox",
        ),
    ] {
        f.server.reset().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/sandboxes/sandbox_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"id":id,"owner":owner,"created_at":timestamp,"status":"active"}),
            ))
            .mount(&f.server)
            .await;
        let result = f
            .admit(
                &f.target,
                "/api/v1/sandboxes/sandbox_1/pause",
                "example-key",
                b"{}",
            )
            .await;
        assert!(
            matches!(result,Err(AppError::NotFound(ref message)) if message == "Sandbox was not created through NyxID"),
            "{timestamp:?} {id}"
        );
    }
    assert_no_managed_rows(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_runtime_enrolling_routes_pass_through() {
    let f = legacy_fixture().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/workspaces/workspace_1"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"id":"workspace_1","sandbox_id":"sandbox_1"})),
        )
        .mount(&f.server)
        .await;
    for (route, body) in [
        (
            "/api/v1/agents",
            r#"{"name":"example","workspace_id":"workspace_1","initial_message":"example","profile":{"permission_mode":"guarded"}}"#,
        ),
        ("/api/v1/agents", r#"{"sandbox_id":"sandbox_1"}"#),
        ("/api/v1/workspaces/workspace_1/agents", "{}"),
        ("/api/v1/sandboxes/sandbox_1/revive", "{}"),
        ("/api/v1/sandboxes/sandbox_1/workspaces", "{}"),
        ("/api/v1/sandboxes/sandbox_1/workspaces/inline", "{}"),
        ("/api/v1/agents/agent_1/input", "{}"),
        (
            "/api/v1/responses",
            r#"{"conversation":"agent_1","stream":true}"#,
        ),
        (
            "/api/v2/responses",
            r#"{"conversation":"agent_1","stream":true}"#,
        ),
        ("/api/v2/agents/agent_1/responses", r#"{"stream":true}"#),
        (
            "/api/v2/agents/agent_1/responses/response_1/interrupt",
            r#"{"turn_sequence":1,"expected_agent_revision":1,"text":"example"}"#,
        ),
    ] {
        let (status, content_type, response_body) = if route.contains("responses")
            && !route.ends_with("interrupt")
        {
            (
                200,
                "text/event-stream",
                "data: {\"id\":\"resp_example\",\"status\":\"in_progress\"}\n\ndata: [DONE]\n\n",
            )
        } else {
            (
                202,
                "application/json",
                "{ \"id\": \"operation_example\", \"status\": \"queued\" }",
            )
        };
        Mock::given(method("POST"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(status).set_body_raw(response_body, content_type))
            .mount(&f.server)
            .await;
        let result = f
            .admit(&f.target, route, "example-key", body.as_bytes())
            .await;
        let permit = result.expect("approved prospective runtime enrollment passes through");
        assert!(
            permit.creation.is_none() && permit.sandbox.is_none(),
            "{route}"
        );
        let request = f.request(route, "example-key", body.as_bytes());
        permit.before_send(&request).await.unwrap();
        let response = permit
            .observe_response(reqwest::Client::new().execute(request).await.unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response.bytes().await.unwrap(), response_body);
        assert_eq!(
            f.server
                .received_requests()
                .await
                .unwrap()
                .last()
                .unwrap()
                .body,
            body.as_bytes()
        );
    }
    assert_eq!(f.post_count().await, 11);
    assert_no_managed_rows(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_miss_command_budget() {
    let mut f = legacy_fixture().await;
    let recorded = monitored(&mut f).await;
    let mut counts = Vec::new();
    for (verb, route, body) in [
        ("POST", "/api/v1/sandboxes/sandbox_1/pause", "{}"),
        ("POST", "/api/v1/sandboxes/sandbox_1/revive", "{}"),
        ("DELETE", "/api/v1/sandboxes/sandbox_1", ""),
        ("POST", "/api/v1/sandboxes/sandbox_1/workspaces", "{}"),
        (
            "POST",
            "/api/v1/sandboxes/sandbox_1/workspaces/inline",
            "{}",
        ),
        ("POST", "/api/v1/agents", r#"{"sandbox_id":"sandbox_1"}"#),
        (
            "POST",
            "/api/v1/agents",
            r#"{"workspace_id":"workspace_1"}"#,
        ),
        ("POST", "/api/v1/workspaces/workspace_1/agents", "{}"),
        ("POST", "/api/v1/agents/agent_1/stop", "{}"),
        ("DELETE", "/api/v1/agents/agent_1", ""),
        ("POST", "/api/v1/agents/agent_1/files", "{}"),
        ("POST", "/api/v1/agents/agent_1/input", "{}"),
        (
            "POST",
            "/api/v1/responses",
            r#"{"conversation":"agent_1","stream":true}"#,
        ),
        (
            "POST",
            "/api/v2/responses",
            r#"{"conversation":"agent_1","stream":true}"#,
        ),
        (
            "POST",
            "/api/v2/agents/agent_1/responses",
            r#"{"stream":true}"#,
        ),
        (
            "POST",
            "/api/v2/agents/agent_1/responses/response_1/controls",
            r#"{"turn_sequence":1,"expected_agent_revision":1,"command":{"type":"cancel"}}"#,
        ),
        (
            "POST",
            "/api/v2/agents/agent_1/responses/response_1/interrupt",
            r#"{"turn_sequence":1,"expected_agent_revision":1,"text":"example"}"#,
        ),
    ] {
        for (key, query) in [
            (true, None),
            (false, None),
            (true, Some("example=1")),
            (false, Some("example=1")),
        ] {
            recorded.lock().unwrap().clear();
            let before = f.server.received_requests().await.unwrap().len();
            let headers = if key {
                vec![("Idempotency-Key".into(), "example-key".into())]
            } else {
                vec![]
            };
            let result = permit_for(&f, verb, route, body.as_bytes(), &headers, query).await;
            let commands = recorded.lock().unwrap().clone();
            let reads = f.server.received_requests().await.unwrap().len() - before;
            counts.push(serde_json::json!({"method":verb,"route":route,"body":body,"key":key,"query":query,"commands":commands.len(),"gateway_reads":reads,"passed":result.is_ok()}));
            assert!(commands.iter().all(|c| c.contains_key("find")));
            assert!(result.is_ok(), "{verb} {route} key={key} query={query:?}");
            let workspace =
                route.starts_with("/api/v1/workspaces/") || body.contains("workspace_id");
            let create = route == "/api/v1/agents" || route.starts_with("/api/v1/workspaces/");
            let agent = !create
                && (route.starts_with("/api/v1/agents/")
                    || route.starts_with("/api/v2/agents/")
                    || route.ends_with("/responses"));
            let managed_validation_passes = query.is_none() && (key || route.ends_with("/files"));
            let expected_reads = if workspace || agent { 2 } else { 1 };
            let expected_commands = 8
                + usize::from(workspace || agent) * 2
                + usize::from(create && managed_validation_passes);
            assert_eq!(
                reads, expected_reads,
                "{verb} {route} key={key} query={query:?}"
            );
            assert_eq!(
                commands.len(),
                expected_commands,
                "{verb} {route} key={key} query={query:?}"
            );
            let mut url = url::Url::parse(&format!("{}{route}", f.server.uri())).unwrap();
            url.set_query(query);
            let mut request = reqwest::Client::new()
                .request(verb.parse().unwrap(), url)
                .body(body.to_owned());
            if key {
                request = request.header("idempotency-key", "example-key");
            }
            recorded.lock().unwrap().clear();
            result
                .unwrap()
                .before_send(&request.build().unwrap())
                .await
                .unwrap();
            let final_commands = recorded.lock().unwrap().clone();
            assert_eq!(final_commands.len(), 2);
            assert!(
                final_commands
                    .iter()
                    .all(|command| command.contains_key("find"))
            );
            assert_eq!(
                f.server.received_requests().await.unwrap().len() - before,
                reads
            );
            counts.last_mut().unwrap()["final_guard_commands"] =
                serde_json::json!(final_commands.len());
        }
    }
    std::fs::write(
        "/tmp/cma2587-green-miss-counts.json",
        serde_json::to_string_pretty(&counts).unwrap(),
    )
    .unwrap();
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn unrelated_proxy_adds_no_commands_or_gateway_reads() {
    let mut f = legacy_fixture().await;
    let recorded = monitored(&mut f).await;
    let mut target = Fixture::connection_target(f.catalog.clone());
    target.catalog_slug = Some("example-service".into());
    registry::admit(
        &f.db,
        &f.keys,
        &f.jwt,
        &f.config,
        &registry::DispatchCaller {
            actor_user_id: &f.owner,
            resource_owner_id: &f.owner,
            user_service_id: None,
        },
        true,
        &target,
        "POST",
        "/example",
        None,
        b"example",
        || panic!("unrelated header work"),
        false,
    )
    .await
    .unwrap();
    assert!(recorded.lock().unwrap().is_empty());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_missing_or_unreadable_marker_is_the_old_replica_refusal() {
    let f = legacy_fixture().await;
    for replacement in [
        None,
        Some(doc! {"_id":MARKER,"rule":"unknown","cutoff":"not-a-date"}),
    ] {
        f.db.collection::<bson::Document>(MARKERS)
            .delete_one(doc! {"_id":MARKER})
            .await
            .unwrap();
        if let Some(row) = replacement {
            f.db.collection::<bson::Document>(MARKERS)
                .insert_one(row)
                .await
                .unwrap();
        }
        let result = f
            .admit(
                &f.target,
                "/api/v1/sandboxes/sandbox_1/pause",
                "example-key",
                b"{}",
            )
            .await;
        assert!(
            matches!(result,Err(AppError::NotFound(ref message)) if message == "Sandbox was not created through NyxID")
        );
        assert!(f.server.received_requests().await.unwrap().is_empty());
    }
    assert_no_managed_rows(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_foreign_agent_and_conflicting_owner_proof_are_not_found() {
    let f = legacy_fixture().await;
    f.server.reset().await;
    Mock::given(method("GET")).and(path("/api/v1/agents/agent_1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id":"agent_1","owner":"foreign-owner","sandbox_id":"sandbox_1","status":"ready"})))
        .mount(&f.server).await;
    let result = f
        .admit(
            &f.target,
            "/api/v1/agents/agent_1/stop",
            "example-key",
            b"{}",
        )
        .await;
    assert!(matches!(result, Err(AppError::NotFound(_))));
    f.server.reset().await;
    Mock::given(method("GET")).and(path("/api/v1/sandboxes/sandbox_1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id":"sandbox_1","owner":f.owner,"owner_id":"foreign-owner","created_at":"2026-01-01T00:00:00Z","status":"active"})))
        .mount(&f.server).await;
    assert!(matches!(
        f.admit(
            &f.target,
            "/api/v1/sandboxes/sandbox_1/pause",
            "example-key",
            b"{}"
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    assert_no_managed_rows(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_does_not_expand_caller_transport_or_operation_scope() {
    let f = legacy_fixture().await;
    for (personal, unsupported_transport) in [(false, false), (true, true)] {
        assert!(
            admit(
                &f.db,
                &f.keys,
                &f.jwt,
                &f.config,
                &f.owner,
                personal,
                &f.target,
                "POST",
                "/api/v1/sandboxes/sandbox_1/pause",
                None,
                b"{}",
                &[],
                unsupported_transport
            )
            .await
            .is_err()
        );
    }
    for route in [
        "/api/v1/launch",
        "/api/v1/visitor",
        "/api/v1/sandboxes/sandbox_1/unknown",
    ] {
        assert!(
            f.admit(&f.target, route, "example-key", b"{}")
                .await
                .is_err()
        );
    }
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert_no_managed_rows(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_production_workspace_creation_is_forwarded_verbatim() {
    let f = legacy_fixture().await;
    let body=br#"{ "name": "example", "workspace_id": "workspace_1", "initial_message": "example", "profile": {"permission_mode": "guarded"} }"#;
    Mock::given(method("POST"))
        .and(path("/api/v1/agents"))
        .respond_with(
            ResponseTemplate::new(202)
                .insert_header("x-cma-test", "unchanged")
                .set_body_raw(
                    "{ \"id\": \"operation_1\", \"status\": \"queued\" }",
                    "application/json",
                ),
        )
        .mount(&f.server)
        .await;
    let caller = registry::DispatchCaller {
        actor_user_id: &f.owner,
        resource_owner_id: &f.owner,
        user_service_id: None,
    };
    let admission = registry::admit(
        &f.db,
        &f.keys,
        &f.jwt,
        &f.config,
        &caller,
        true,
        &f.target,
        "POST",
        "/api/v1/agents",
        None,
        body,
        || Ok(vec![].into()),
        false,
    )
    .await
    .expect("production workspace-only request must pass");
    let response = crate::services::proxy_service::forward_request_with_resource_proof(
        crate::services::proxy_service::Redirects::Return,
        &f.target,
        reqwest::Method::POST,
        "/api/v1/agents",
        None,
        reqwest::header::HeaderMap::new(),
        crate::services::proxy_service::ProxyBody::Buffered(Some(bytes::Bytes::copy_from_slice(
            body,
        ))),
        vec![],
        vec![],
        None,
        &crate::services::provider_token_exchange_service::TokenExchangeCache::new(),
        &crate::services::cloud_response_cache::CloudResponseCache::new(0),
        vec![],
        crate::services::billing::route_inventory::internal_node_dispatch_permit(),
        None,
        None,
        Some(&admission),
        Some(&caller),
        None,
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 202);
    assert_eq!(response.headers()["x-cma-test"], "unchanged");
    assert_eq!(
        response.bytes().await.unwrap(),
        "{ \"id\": \"operation_1\", \"status\": \"queued\" }"
    );
    let requests = f.server.received_requests().await.unwrap();
    assert_eq!(requests.last().unwrap().body, body);
    assert!(
        !requests
            .last()
            .unwrap()
            .headers
            .contains_key("idempotency-key")
    );
    for request in requests.iter().filter(|r| r.method.as_str() == "GET") {
        let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
        validation.set_audience(&[f
            .catalog
            .identity_jwt_audience
            .as_deref()
            .unwrap_or(&f.catalog.base_url)]);
        validation.validate_exp = false;
        let token = request.headers["x-nyxid-identity-token"].to_str().unwrap();
        let claims = jsonwebtoken::decode::<Value>(token, &f.jwt.decoding, &validation)
            .unwrap()
            .claims;
        assert_eq!(claims["sub"], f.owner);
        assert_eq!(claims["nyx_service_id"], f.catalog.id);
    }
    assert_no_managed_rows(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_marker_is_initialized_before_startup_serves_requests() {
    let f = Fixture::new().await;
    crate::db::initialize(&f.db).await.unwrap();
    let marker =
        f.db.collection::<bson::Document>(MARKERS)
            .find_one(doc! {"_id":MARKER})
            .await
            .unwrap();
    assert!(
        marker.is_some(),
        "startup must persist the deployment cutoff"
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_all_creation_selectors_accept_keyless_queries_but_tracked_parents_keep_validation()
{
    let mut f = legacy_fixture().await;
    let recorded = monitored(&mut f).await;
    for (route, body) in [
        (
            "/api/v1/agents",
            r#"{ "sandbox_id": "sandbox_1", "opaque": {"value":true} }"#,
        ),
        (
            "/api/v1/agents",
            r#"{ "workspace_id": "workspace_1", "opaque": {"value":true} }"#,
        ),
        (
            "/api/v1/workspaces/workspace_1/agents",
            r#"{ "opaque": {"value":true} }"#,
        ),
    ] {
        for query in [None, Some("b=%2F&a=1&a=2")] {
            let permit = permit_for(&f, "POST", route, body.as_bytes(), &[], query)
                .await
                .unwrap();
            let mut url = url::Url::parse(&format!("{}{route}", f.server.uri())).unwrap();
            url.set_query(query);
            let request = reqwest::Client::new()
                .post(url)
                .body(body.to_owned())
                .build()
                .unwrap();
            permit.before_send(&request).await.unwrap();
            assert_eq!(request.url().query(), query);
            assert_eq!(request.body().unwrap().as_bytes().unwrap(), body.as_bytes());
            assert!(permit.creation.is_none() && permit.sandbox.is_none());
        }
    }
    assert_no_managed_rows(&f).await;
    tracked_parent(&f).await;
    let collection =
        f.db.collection::<bson::Document>(crate::models::assistant_cma_sandbox::COLLECTION_NAME);
    let before = bson::to_vec(&collection.find_one(doc! {}).await.unwrap().unwrap()).unwrap();
    for (route, body) in [
        ("/api/v1/agents", r#"{"sandbox_id":"sandbox_1"}"#),
        ("/api/v1/agents", r#"{"workspace_id":"workspace_1"}"#),
        ("/api/v1/workspaces/workspace_1/agents", "{}"),
        ("/api/v1/sandboxes/sandbox_1/pause", "{}"),
        ("/api/v1/agents/agent_1/stop", "{}"),
    ] {
        for query in [None, Some("example=1")] {
            recorded.lock().unwrap().clear();
            let reads_before = f.server.received_requests().await.unwrap().len();
            assert!(matches!(
                permit_for(&f, "POST", route, body.as_bytes(), &[], query).await,
                Err(AppError::BadRequest(_))
            ));
            let commands = recorded.lock().unwrap().clone();
            let workspace =
                route.starts_with("/api/v1/workspaces/") || body.contains("workspace_id");
            let agent = route.starts_with("/api/v1/agents/");
            assert_eq!(
                commands.len(),
                if workspace {
                    9
                } else if agent {
                    7
                } else {
                    5
                }
            );
            assert_eq!(
                f.server.received_requests().await.unwrap().len() - reads_before,
                if workspace { 2 } else { usize::from(agent) }
            );
            assert!(commands.iter().all(|command| command.contains_key("find")));
            assert!(!normalized_commands(&commands, &f).contains(MARKERS));
            assert_eq!(
                bson::to_vec(&collection.find_one(doc! {}).await.unwrap().unwrap()).unwrap(),
                before
            );
        }
    }
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_nanosecond_boundary_and_all_missing_malformed_shared_proofs() {
    let f = legacy_fixture().await;
    for timestamp in [
        serde_json::json!("2026-01-01T23:59:59.999999999Z"),
        serde_json::json!("2026-01-02T07:59:59.999999999+08:00"),
    ] {
        f.server.reset().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/sandboxes/sandbox_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"id":"sandbox_1","owner":f.owner,"created_at":timestamp}),
            ))
            .mount(&f.server)
            .await;
        assert!(
            f.admit(
                &f.target,
                "/api/v1/sandboxes/sandbox_1/pause",
                "example-key",
                b"{}"
            )
            .await
            .is_ok()
        );
    }
    for changes in [
        serde_json::json!({"created_at":null}),
        serde_json::json!({"created_at":123}),
        serde_json::json!({"created_at":{}}),
        serde_json::json!({"created_at":"2026-01-02T08:00:00+08:00"}),
        serde_json::json!({"created_at":"2026-01-01T00:00:00"}),
        serde_json::json!({"owner":null}),
        serde_json::json!({"owner":f.owner,"owner_id":"foreign-owner"}),
        serde_json::json!({"owner":f.owner,"user_id":"foreign-owner"}),
        serde_json::json!({"access":"shared"}),
        serde_json::json!({"shared":true}),
        serde_json::json!({"sandbox_id":"different_sandbox"}),
    ] {
        f.server.reset().await;
        let mut payload = serde_json::json!({"id":"sandbox_1","owner":f.owner,"created_at":"2026-01-01T00:00:00Z"});
        payload
            .as_object_mut()
            .unwrap()
            .extend(changes.as_object().unwrap().clone());
        Mock::given(method("GET"))
            .and(path("/api/v1/sandboxes/sandbox_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(payload))
            .mount(&f.server)
            .await;
        assert!(
            matches!(f.admit(&f.target,"/api/v1/sandboxes/sandbox_1/pause","example-key",b"{}").await,
            Err(AppError::NotFound(ref message)) if message=="Sandbox was not created through NyxID")
        );
    }
    assert_no_managed_rows(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_gateway_outage_and_marker_read_error_never_forward() {
    let f = legacy_fixture().await;
    f.server.reset().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/sandboxes/sandbox_1"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&f.server)
        .await;
    assert!(matches!(
        f.admit(
            &f.target,
            "/api/v1/sandboxes/sandbox_1/pause",
            "example-key",
            b"{}"
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    assert_eq!(f.post_count().await, 0);
    f.db.collection::<bson::Document>(MARKERS)
        .drop()
        .await
        .unwrap();
    f.db.collection::<bson::Document>("marker_read_failure_source")
        .insert_one(doc! {"_id":MARKER})
        .await
        .unwrap();
    f.db.run_command(doc!{"create":MARKERS,"viewOn":"marker_read_failure_source","pipeline":[{"$set":{"cutoff":{"$divide":[1,0]}}}]}).await.unwrap();
    assert!(
        f.db.collection::<bson::Document>(MARKERS)
            .find_one(doc! {"_id":MARKER})
            .await
            .is_err()
    );
    let before = f.server.received_requests().await.unwrap().len();
    assert!(matches!(
        f.admit(
            &f.target,
            "/api/v1/sandboxes/sandbox_1/pause",
            "example-key",
            b"{}"
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    assert_eq!(f.server.received_requests().await.unwrap().len(), before);
    assert_no_managed_rows(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_new_replica_and_old_tracked_only_boundary_share_no_managed_state() {
    let f = legacy_fixture().await;
    let peer = mongodb::Client::with_uri_str(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let old_db = peer.database(f.db.name());
    let old_gateway = CatalogCmaGateway::from_parts(&old_db, &f.jwt, &f.config, &f.catalog.id);
    let original = bson::to_vec(
        &f.db
            .collection::<bson::Document>(MARKERS)
            .find_one(doc! {"_id":MARKER})
            .await
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    // Old replicas stop at this same tracked-only resolver. Keep it callable by
    // first-party management as well as this independent-client rollout check.
    let (old, new) = tokio::join!(
        sandboxes::resolve_created(&old_db, &old_gateway, &f.owner, "sandbox_1"),
        f.admit(
            &f.target,
            "/api/v1/sandboxes/sandbox_1/pause",
            "example-key",
            b"{}"
        ),
    );
    assert!(
        matches!(old,Err(AppError::NotFound(ref message)) if message=="Sandbox was not created through NyxID")
    );
    let new = new.unwrap();
    assert!(new.creation.is_none() && new.sandbox.is_none());
    new.before_send(&f.request("/api/v1/sandboxes/sandbox_1/pause", "example-key", b"{}"))
        .await
        .unwrap();
    assert!(matches!(
        sandboxes::resolve_created(&old_db, &old_gateway, &f.owner, "sandbox_1").await,
        Err(AppError::NotFound(_))
    ));
    assert_no_managed_rows(&f).await;
    assert_eq!(
        bson::to_vec(
            &f.db
                .collection::<bson::Document>(MARKERS)
                .find_one(doc! {"_id":MARKER})
                .await
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        original
    );
    let list = sandboxes::list(&f.db, &f.owner, None).await.unwrap();
    assert!(list.sandboxes.is_empty());
    assert_eq!(list.occupied, 0);
    assert!(matches!(
        crate::services::assistant_cma_agent_service::get_owned(&f.db, &f.owner, "agent_1").await,
        Err(AppError::NotFound(_))
    ));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn cma_read_methods_do_not_consult_provenance_or_cutoff() {
    let mut f = legacy_fixture().await;
    let recorded = monitored(&mut f).await;
    for verb in ["GET", "HEAD", "OPTIONS"] {
        recorded.lock().unwrap().clear();
        let permit = permit_for(&f, verb, "/api/v1/agents", b"", &[], Some("example=1"))
            .await
            .unwrap();
        let request = reqwest::Client::new()
            .request(
                verb.parse().unwrap(),
                format!("{}/api/v1/agents?example=1", f.server.uri()),
            )
            .build()
            .unwrap();
        permit.before_send(&request).await.unwrap();
        let commands = recorded.lock().unwrap().clone();
        assert_eq!(commands.len(), 4);
        assert!(!normalized_commands(&commands, &f).contains(MARKER));
        assert!(f.server.received_requests().await.unwrap().is_empty());
    }
    assert_no_managed_rows(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_watch_agent_registration_keeps_managed_collections_empty() {
    let f = legacy_fixture().await;
    let source = crate::services::assistant_cma_agent_service::AgentSource {
        watch_id: Some("watch_example".into()),
        creation_operation_id: Some("operation_example".into()),
        ..Default::default()
    };
    assert!(matches!(
        crate::services::assistant_cma_agent_service::register_live(
            &f.db,
            &f.gateway(),
            &f.owner,
            "agent_1",
            Some(&source)
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    assert!(
        crate::services::cma_operation::register_observed_agent(
            &f.db,
            &f.gateway(),
            &f.owner,
            "agent_1",
            &source
        )
        .await
        .unwrap()
        .is_none()
    );
    assert_no_managed_rows(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_parent_does_not_reuse_or_mutate_an_unrelated_managed_receipt() {
    let f = legacy_fixture().await;
    let managed = f
        .admit(
            &f.target,
            "/api/v1/sandboxes",
            "shared-client-key",
            br#"{"machine_included":false}"#,
        )
        .await
        .unwrap();
    assert!(managed.creation.is_some());
    let creations = f.db.collection::<bson::Document>(COLLECTION_NAME);
    let sandboxes =
        f.db.collection::<bson::Document>(crate::models::assistant_cma_sandbox::COLLECTION_NAME);
    let before_creation =
        bson::to_vec(&creations.find_one(doc! {}).await.unwrap().unwrap()).unwrap();
    let before_sandbox =
        bson::to_vec(&sandboxes.find_one(doc! {}).await.unwrap().unwrap()).unwrap();
    let legacy = f
        .admit(
            &f.target,
            "/api/v1/agents",
            "shared-client-key",
            br#"{"workspace_id":"workspace_1"}"#,
        )
        .await
        .unwrap();
    assert!(legacy.creation.is_none() && legacy.sandbox.is_none());
    legacy
        .before_send(&f.request(
            "/api/v1/agents",
            "shared-client-key",
            br#"{"workspace_id":"workspace_1"}"#,
        ))
        .await
        .unwrap();
    assert_eq!(
        bson::to_vec(&creations.find_one(doc! {}).await.unwrap().unwrap()).unwrap(),
        before_creation
    );
    assert_eq!(
        bson::to_vec(&sandboxes.find_one(doc! {}).await.unwrap().unwrap()).unwrap(),
        before_sandbox
    );
    f.assert_single_intent().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_workspace_and_agent_selectors_reject_unowned_shared_or_undated_parents() {
    let f = legacy_fixture().await;
    for changes in [
        serde_json::json!({"owner":"foreign-owner"}),
        serde_json::json!({"access":"shared"}),
        serde_json::json!({"created_at":null}),
        serde_json::json!({"created_at":"2026-01-02T00:00:00Z"}),
    ] {
        let mut parent = serde_json::json!({"id":"sandbox_1","owner":f.owner,"created_at":"2026-01-01T00:00:00Z"});
        parent
            .as_object_mut()
            .unwrap()
            .extend(changes.as_object().unwrap().clone());
        let scoped = Mock::given(method("GET"))
            .and(path("/api/v1/sandboxes/sandbox_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(parent))
            .with_priority(1)
            .mount_as_scoped(&f.server)
            .await;
        for (route, body) in [
            (
                "/api/v1/agents",
                br#"{"workspace_id":"workspace_1"}"#.as_slice(),
            ),
            ("/api/v1/workspaces/workspace_1/agents", b"{}".as_slice()),
            ("/api/v1/agents/agent_1/input", b"{}".as_slice()),
        ] {
            assert!(
                matches!(
                    f.admit(&f.target, route, "example-key", body).await,
                    Err(AppError::NotFound(_))
                ),
                "{route}"
            );
        }
        drop(scoped);
    }
    assert_no_managed_rows(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_parent_authorization_refusals_remain_not_found_shaped() {
    let f = legacy_fixture().await;
    for (detail, route, body) in [
        (
            "/api/v1/agents/agent_1",
            "/api/v1/agents/agent_1/input",
            b"{}".as_slice(),
        ),
        (
            "/api/v1/workspaces/workspace_1",
            "/api/v1/agents",
            br#"{"workspace_id":"workspace_1"}"#.as_slice(),
        ),
        (
            "/api/v1/sandboxes/sandbox_1",
            "/api/v1/agents",
            br#"{"workspace_id":"workspace_1"}"#.as_slice(),
        ),
    ] {
        for status in [401, 403, 404] {
            let scoped = Mock::given(method("GET"))
                .and(path(detail))
                .respond_with(ResponseTemplate::new(status))
                .with_priority(1)
                .mount_as_scoped(&f.server)
                .await;
            assert!(
                matches!(
                    f.admit(&f.target, route, "example-key", body).await,
                    Err(AppError::NotFound(_))
                ),
                "{detail}: {status}"
            );
            drop(scoped);
        }
    }
    assert_no_managed_rows(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_targeted_control_variants_preserve_the_request_and_upstream_refusal() {
    let f = legacy_fixture().await;
    let route = "/api/v2/agents/agent_1/responses/response_1/controls";
    Mock::given(method("POST"))
        .and(path(route))
        .respond_with(ResponseTemplate::new(409).set_body_raw(
            "{ \"code\": \"upstream_control_conflict\" }",
            "application/problem+json",
        ))
        .mount(&f.server)
        .await;
    for command in [
        serde_json::json!({"type":"cancel"}),
        serde_json::json!({"type":"steer","text":"example"}),
        serde_json::json!({"type":"resolve_approval","approval_id":"approval_example","decision":"allow"}),
        serde_json::json!({"type":"resolve_approval","approval_id":"approval_example","decision":"deny"}),
        serde_json::json!({"type":"resolve_approval","approval_id":"approval_example","decision":"abort"}),
    ] {
        let body = serde_json::to_vec(
            &serde_json::json!({"turn_sequence":1,"expected_agent_revision":1,"command":command}),
        )
        .unwrap();
        let permit = f
            .admit(&f.target, route, "control-key", &body)
            .await
            .unwrap();
        let request = f.request(route, "control-key", &body);
        permit.before_send(&request).await.unwrap();
        let response = permit
            .observe_response(reqwest::Client::new().execute(request).await.unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), 409);
        assert_eq!(
            response.bytes().await.unwrap(),
            "{ \"code\": \"upstream_control_conflict\" }"
        );
        assert_eq!(
            f.server
                .received_requests()
                .await
                .unwrap()
                .last()
                .unwrap()
                .body,
            body
        );
    }
    assert_no_managed_rows(&f).await;
    f.db.drop().await.unwrap();
}

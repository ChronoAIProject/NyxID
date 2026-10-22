//! Real replica-set funding and loopback provider contract, no provider entitlement claim.
use super::*;
use crate::models::{
    assistant_voice::WINDOWS,
    usage_meter::{COLLECTION_NAME as USAGE, UsageMeterRow},
};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
async fn dictation_checkpoint_carry_replay_and_stale_owner_cannot_seal_current_usage() {
    let (f, service) = super::super::credential_tests::fixture("dictation_fenced_money").await;
    let p = super::super::credential_tests::platform(&service);
    let resolved = resolve(&f.state, &f.owner, &f.row.id, &p).await.unwrap();
    let call = session::admit_dictation(
        &f.state.db,
        &f.owner,
        &f.row.id,
        &uuid::Uuid::new_v4().to_string(),
        p,
        resolved.identity,
        "worker",
    )
    .await
    .unwrap();
    let meter = crate::services::billing::voice::reserve(
        &f.state.db,
        &f.state.billing,
        resolved.billing,
        &call.id,
        0,
    )
    .await
    .unwrap();
    f.state.billing.mark_forwarded(&meter).await.unwrap();
    let stale = session::write(
        &f.state.db,
        &call,
        doc! {"state":"active","reserved_until":30},
    )
    .await
    .unwrap();
    let latest = checkpoint(&f.state, &stale, "A", 600_000_000_000)
        .await
        .unwrap();
    assert_eq!(latest.observed_seconds, 0);
    let latest = checkpoint(&f.state, &latest, "B", 600_000_000_000)
        .await
        .unwrap();
    assert_eq!(latest.observed_seconds, 1);
    assert_eq!(
        checkpoint(&f.state, &latest, "B", 600_000_000_000)
            .await
            .unwrap()
            .observed_seconds,
        1
    );
    assert!(
        checkpoint(&f.state, &latest, "B", 700_000_000_000)
            .await
            .is_err()
    );
    // Present corrupt money state must never be interpreted as a new/empty checkpoint.
    let rows = f.state.db.collection::<bson::Document>(COLLECTION_NAME);
    for (field, corrupt) in [
        (
            "dictation_duration_pico",
            bson::Bson::String("invalid".into()),
        ),
        ("dictation_items", bson::Bson::Null),
    ] {
        let saved = rows
            .find_one(doc! {"_id":&call.id})
            .await
            .unwrap()
            .unwrap()
            .get(field)
            .cloned()
            .unwrap();
        rows.update_one(doc! {"_id":&call.id}, doc! {"$set":{field:corrupt}})
            .await
            .unwrap();
        assert!(checkpoint(&f.state, &latest, "C", PICO).await.is_err());
        rows.update_one(doc! {"_id":&call.id}, doc! {"$set":{field:saved}})
            .await
            .unwrap();
    }
    // A new replica claims the generation; stale owner must not seal any window.
    f.state
        .db
        .collection::<bson::Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":&call.id},
            doc! {"$inc":{"generation":1},"$set":{"lease_owner":"new-owner"}},
        )
        .await
        .unwrap();
    assert!(finish(&f.state, &stale, false).await.is_err());
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(WINDOWS)
            .count_documents(doc! {"session_id":&call.id,"sealed":true})
            .await
            .unwrap(),
        0
    );
    let current = session::get(&f.state.db, &f.owner, &f.row.id, &call.id)
        .await
        .unwrap();
    let closing = session::write(&f.state.db, &current, doc! {"state":"closing"})
        .await
        .unwrap();
    assert!(checkpoint(&f.state, &closing, "C", PICO).await.is_err());
    finish(&f.state, &closing, false).await.unwrap();
    assert!(finish(&f.state, &stale, false).await.is_err());
    let windows: Vec<_> = f
        .state
        .db
        .collection::<bson::Document>(WINDOWS)
        .find(doc! {"session_id":&call.id})
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await;
    assert_eq!(windows.len(), 1);
    let usage = f
        .state
        .db
        .collection::<UsageMeterRow>(USAGE)
        .find_one(
            doc! {"billing_request_id":format!("voice:{}:0",call.id),"metric":"voice_seconds"},
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(usage.quantity, Some(1));
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
            .count_documents(doc! {"voice.session_id":&call.id})
            .await
            .unwrap(),
        0,
        "dictation writes no transcript or call receipt to chat"
    );
}

#[test]
fn dictation_ready_requires_transcription_model_and_format() {
    let mut event = configuration();
    event["type"] = json!("session.updated");
    assert!(valid_configuration(&event));
    for (path, value) in [
        ("type", json!("realtime")),
        ("audio", json!({})),
        ("tools", json!([{"name":"execute"}])),
    ] {
        let mut wrong = event.clone();
        wrong["session"][path] = value;
        assert!(!valid_configuration(&wrong));
    }
    let mut wrong = event.clone();
    wrong["session"]["audio"]["input"]["transcription"]["model"] = json!("other");
    assert!(!valid_configuration(&wrong));
}
#[tokio::test(start_paused = true)]
async fn dictation_duration_limit_and_remote_end_start_fresh_drain_deadline() {
    let mut limit = Drain(None);
    tokio::time::advance(std::time::Duration::from_secs(120)).await;
    limit.stop();
    assert!(!limit.expired());
    tokio::time::advance(std::time::Duration::from_secs(10)).await;
    limit.stop();
    assert!(
        !limit.expired(),
        "repeat stop must not reset or predate drain"
    );
    tokio::time::advance(std::time::Duration::from_secs(3)).await;
    assert!(limit.expired());
    let mut remote = Drain(None);
    remote.stop();
    assert!(!remote.expired());
}

async fn loopback(bad_usage: bool, disconnect: bool, flood: bool) {
    let (f, service) = super::super::credential_tests::fixture(if bad_usage {
        "dictation_bad_usage"
    } else if disconnect {
        "dictation_disconnect"
    } else {
        "dictation_relay"
    })
    .await;
    let preferences = super::super::credential_tests::platform(&service);
    let human = uuid::Uuid::new_v4().to_string();
    f.state.db.collection::<bson::Document>(crate::models::session::COLLECTION_NAME).insert_one(doc!{"_id":&human,"user_id":&f.owner,"revoked":false,"expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::minutes(5))}).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let provider = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        let config = ws.next().await.unwrap().unwrap();
        let actual: Value = serde_json::from_str(config.to_text().unwrap()).unwrap();
        assert_eq!(actual, configuration());
        let mut echo = actual;
        echo["type"] = json!("session.updated");
        ws.send(Message::Text(echo.to_string().into()))
            .await
            .unwrap();
        let mut commits = 0;
        let mut noise = tokio::time::interval(std::time::Duration::from_millis(2));
        loop {
            let message = tokio::select! {
                next=ws.next()=> { let Some(Ok(message))=next else {break}; message },
                _=noise.tick(), if flood => {
                    for _ in 0..32 { if ws.send(Message::Text(json!({"type":"rate_limits.updated"}).to_string().into())).await.is_err() {return;} }
                    continue;
                }
            };
            let Message::Text(text) = message else {
                continue;
            };
            let event: Value = serde_json::from_str(&text).unwrap();
            match event["type"].as_str().unwrap() {
                "input_audio_buffer.append" => {
                    assert!(event.get("audio").is_some());
                }
                "input_audio_buffer.commit" => {
                    commits += 1;
                    let id = if commits == 1 { "A" } else { "B" };
                    ws.send(Message::Text(
                        json!({"type":"input_audio_buffer.committed","item_id":id})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                    if commits == 1 {
                        // B can begin live before A's no-delta completion arrives.
                        ws.send(Message::Text(json!({"type":"conversation.item.input_audio_transcription.delta","item_id":"B","delta":"second interim"}).to_string().into())).await.unwrap();
                    }
                    let completion = json!({"type":"conversation.item.input_audio_transcription.completed","item_id":id,"transcript":if commits==1 {"first"}else{"second"},
                        "usage":if bad_usage && commits==2 {json!({"type":"tokens","total_tokens":5})}else{json!({"type":"duration","seconds":0.6})}});
                    ws.send(Message::Text(completion.to_string().into()))
                        .await
                        .unwrap();
                    if !bad_usage {
                        ws.send(Message::Text(completion.to_string().into()))
                            .await
                            .unwrap();
                    }
                }
                other => panic!("unbounded provider command: {other}"),
            }
        }
        assert_eq!(
            commits, 2,
            "all forwarded buffers must be committed even after UI disconnect"
        );
    });
    let (socket, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let (sender, receiver) = mpsc::channel(8);
    let (output, mut events) = mpsc::channel(32);
    let state = f.state.clone();
    let owner = f.owner.clone();
    let thread = f.row.id.clone();
    let task = tokio::spawn(serve_with_provider(
        state,
        owner,
        thread,
        human,
        uuid::Uuid::new_v4().to_string(),
        preferences,
        receiver,
        output,
        Provider::Fixture(Box::new(socket)),
    ));
    let ready = tokio::time::timeout(std::time::Duration::from_secs(20), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ready["type"], "ready", "{ready}");
    // Five real seconds trigger periodic commit; bounded frames obey ingress pace.
    for _ in 0..125 {
        sender.send(Input::Audio(vec![0; 1920])).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    }
    for _ in 0..15 {
        sender.send(Input::Audio(vec![0; 1920])).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
    }
    if disconnect {
        drop(events);
        drop(sender);
    } else {
        sender.send(Input::Stop).await.unwrap();
        let mut captions = Vec::new();
        let mut terminal = Value::Null;
        while let Some(event) =
            tokio::time::timeout(std::time::Duration::from_secs(20), events.recv())
                .await
                .unwrap()
        {
            if event["type"] == "completed" {
                captions.push(event.clone());
            }
            if event["type"] == "stopped" || event["type"] == "error" {
                terminal = event;
                break;
            }
        }
        assert_eq!(
            terminal["type"],
            if bad_usage { "error" } else { "stopped" },
            "{terminal}"
        );
        assert_eq!(captions[0]["item_id"], "A");
        assert_eq!(captions[0]["order"], 0);
        if !bad_usage {
            assert_eq!(captions[1]["item_id"], "B");
            assert_eq!(captions[1]["order"], 1);
            assert_eq!(captions.len(), 2);
        }
    }
    tokio::time::timeout(std::time::Duration::from_secs(20), task)
        .await
        .unwrap()
        .unwrap();
    provider.await.unwrap();
    let call = f
        .state
        .db
        .collection::<VoiceSession>(COLLECTION_NAME)
        .find_one(doc! {"user_id":&f.owner,"dictation":true})
        .await
        .unwrap()
        .unwrap();
    assert!(!call.live_slot);
    assert!(call.billing_finalized);
    assert_eq!(call.observed_seconds, if bad_usage { 0 } else { 1 });
    assert_eq!(call.final_usage_confirmed, !bad_usage);
    let usage = f
        .state
        .db
        .collection::<UsageMeterRow>(USAGE)
        .find_one(
            doc! {"billing_request_id":format!("voice:{}:0",call.id),"metric":"voice_seconds"},
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(usage.quantity, Some(call.observed_seconds));
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
            .count_documents(doc! {"voice.session_id":&call.id})
            .await
            .unwrap(),
        0
    );
}
#[tokio::test]
async fn dictation_loopback_confirmed_duration_order_dedup_and_stop() {
    loopback(false, false, false).await;
}
#[tokio::test]
async fn dictation_loopback_disconnect_drains_forwarded_audio() {
    loopback(false, true, false).await;
}
#[tokio::test]
async fn dictation_loopback_wrong_usage_stops_without_free_looping() {
    loopback(true, false, false).await;
}

#[tokio::test]
async fn dictation_preserves_bound_platform_authority_and_rejects_byok() {
    let (f, service) = super::super::credential_tests::fixture("dictation_bound_authority").await;
    let mut p = super::super::credential_tests::platform(&service);
    let key = crate::services::key_service::get_api_key(
        &f.state.db,
        &f.owner,
        &f.row.credential_api_key_id,
    )
    .await
    .unwrap();
    resolve(&f.state, &f.owner, &f.row.id, &p).await.unwrap();
    assert_eq!(
        bson::to_document(&key).unwrap(),
        bson::to_document(
            &crate::services::key_service::get_api_key(
                &f.state.db,
                &f.owner,
                &f.row.credential_api_key_id
            )
            .await
            .unwrap()
        )
        .unwrap()
    );
    p.key_source = crate::models::assistant_voice::VoiceKeySource::Own;
    p.connection_id = Some(uuid::Uuid::new_v4().to_string());
    assert!(resolve(&f.state, &f.owner, &f.row.id, &p).await.is_err());
    p.key_source = crate::models::assistant_voice::VoiceKeySource::Platform;
    p.connection_id = None;
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent_credential::COLLECTION_NAME)
        .delete_many(doc! {"api_key_id":&f.row.credential_api_key_id})
        .await
        .unwrap();
    assert!(resolve(&f.state, &f.owner, &f.row.id, &p).await.is_err());
}

#[test]
fn dictation_transcript_fragments_have_bounded_valid_timing() {
    use crate::services::voice::transcript::{MAX_FRAGMENTS, Transcripts};
    let mut t = Transcripts::default();
    assert!(t.ingest(&json!({"type":"session.output_transcript.delta","event_id":"invalid","delta":"bad","start_ms":20,"end_ms":10}),0).is_err());
    for i in 0..MAX_FRAGMENTS {
        let rows=t.ingest(&json!({"type":"session.output_transcript.delta","event_id":format!("f{i}"),"delta":"x","start_ms":i,"end_ms":i+1}),0).unwrap();
        assert_eq!(rows.last().unwrap().fragments.len(), i + 1);
    }
    assert!(t.ingest(&json!({"type":"session.output_transcript.delta","event_id":"overflow","delta":"x","start_ms":256,"end_ms":257}),0).is_err());
    let mut empty = Transcripts::default();
    for i in 0..1000 {
        assert!(empty.ingest(&json!({"type":"session.output_transcript.delta","event_id":format!("e{i}"),"delta":"","start_ms":0,"end_ms":0}),0).unwrap().is_empty());
    }
}

#[tokio::test]
async fn dictation_get_permission_is_independent_of_live_post_permission() {
    let (f, mut service) = super::super::credential_tests::fixture("dictation_operation").await;
    let p = super::super::credential_tests::platform(&service);
    for (method, path, allowed) in [
        ("GET", "/realtime", true),
        ("POST", "/live/sessions", false),
    ] {
        service.proxy_operation_policy =
            Some(crate::models::downstream_service::ProxyOperationPolicy {
                rules: vec![crate::models::downstream_service::ProxyOperationRule {
                    method: method.into(),
                    path_template: path.into(),
                    ..Default::default()
                }],
            });
        f.state
            .db
            .collection::<crate::models::downstream_service::DownstreamService>(
                crate::models::downstream_service::COLLECTION_NAME,
            )
            .replace_one(doc! {"_id":&service.id}, &service)
            .await
            .unwrap();
        assert_eq!(
            resolve(&f.state, &f.owner, &f.row.id, &p).await.is_ok(),
            allowed
        );
        let options = crate::services::assistant_voice::dictation_options(
            &f.state.db,
            &f.state.encryption_keys,
            &f.row,
        )
        .await
        .unwrap();
        assert_eq!(!options.is_empty(), allowed);
        assert_eq!(
            super::credentials::resolve(&f.state, &f.owner, &f.row.id, &p)
                .await
                .is_ok(),
            !allowed,
            "normal voice retains POST permission"
        );
    }
}

#[tokio::test]
async fn dictation_loopback_irrelevant_provider_flood_cannot_starve_stop() {
    loopback(false, false, true).await;
}

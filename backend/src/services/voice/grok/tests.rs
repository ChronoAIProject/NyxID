use super::{arbiter::Arbiter, protocol};
use serde_json::json;

#[test]
fn grok_contract_disables_builtin_tools_vad_and_resumption() {
    let config = protocol::configuration("catalog-model", "catalog-voice", "context");
    let session = &config["session"];
    assert_eq!(session["model"], "catalog-model");
    assert_eq!(session["voice"], "catalog-voice");
    assert!(session["turn_detection"]["type"].is_null());
    assert_eq!(session["resumption"]["enabled"], false);
    assert_eq!(session["tools"].as_array().unwrap().len(), 1);
    assert_eq!(session["tools"][0]["name"], "delegate_to_agent");
    assert_eq!(
        session["tools"][0]["parameters"]["additionalProperties"],
        false
    );
    assert_eq!(
        session["audio"]["input"]["transcription"]["model"],
        "grok-transcribe"
    );
}

#[test]
fn grok_delegate_is_closed_and_never_carries_action_authority() {
    let id = uuid::Uuid::new_v4().to_string();
    let mut event = json!({"name":"delegate_to_agent","arguments":json!({"utterance_id":id,"intent_hint":"lookup"}).to_string()});
    assert!(protocol::delegation(&event).is_some());
    event["arguments"] = json!({"utterance_id":id,"intent_hint":"lookup","approved":true})
        .to_string()
        .into();
    assert!(protocol::delegation(&event).is_none());
    event["name"] = "web_search".into();
    assert!(protocol::delegation(&event).is_none());
}

#[test]
fn grok_arbiter_waits_for_all_receipts_and_playback_and_coalesces_responses() {
    let mut arbiter = Arbiter::default();
    arbiter.respond();
    assert_eq!(arbiter.take_next(), Some(None));
    assert!(arbiter.take_next().is_none());
    arbiter.created("response".into()).unwrap();
    arbiter.playback_end_ms = 500;
    arbiter.unresolved.insert("call1".into());
    arbiter.unresolved.insert("call2".into());
    arbiter.respond();
    arbiter.respond();
    arbiter.done("response").unwrap();
    arbiter.playback_ms = 500;
    arbiter.unresolved.remove("call1");
    assert!(arbiter.take_next().is_none());
    arbiter.unresolved.clear();
    arbiter.playback_ms = 499;
    assert!(arbiter.take_next().is_none());
    arbiter.playback_ms = 500;
    assert_eq!(arbiter.take_next(), Some(None));
    assert!(arbiter.take_next().is_none());
}

#[test]
fn grok_force_message_owns_one_response_lifecycle() {
    let mut arbiter = Arbiter::default();
    arbiter.line("Exact server question".into()).unwrap();
    assert_eq!(
        arbiter.take_next(),
        Some(Some("Exact server question".into()))
    );
    arbiter.created("forced".into()).unwrap();
    arbiter.done("forced").unwrap();
    assert!(arbiter.take_next().is_none());
}

#[tokio::test]
async fn grok_fixture_manual_transcription_during_output_is_cumulative_and_does_not_cancel() {
    Box::pin(async {
        use axum::{Router, routing::get, extract::{WebSocketUpgrade,ws::Message}, http::HeaderMap};
        use crate::services::voice::{grok::relay::{Grok,ClientInput}, transcript::Segment};
        let (sent, mut received)=tokio::sync::mpsc::channel(64);
        let app=Router::new().route("/v1/realtime",get(move |headers:HeaderMap,ws:WebSocketUpgrade| {
            let sent=sent.clone();
            async move {
                assert_eq!(headers["authorization"],"Bearer fixture-secret");
                ws.on_upgrade(move |mut socket| async move {
                    socket.send(Message::Text(json!({"type":"session.created"}).to_string().into())).await.unwrap();
                    let Some(Ok(Message::Text(text)))=socket.recv().await else {panic!("configuration required")};
                    let config:serde_json::Value=serde_json::from_str(&text).unwrap();
                    assert_eq!(config["session"]["tools"].as_array().unwrap().len(),1);
                    socket.send(Message::Text(json!({"type":"session.updated","session":config["session"]}).to_string().into())).await.unwrap();
                    while let Some(Ok(Message::Text(text)))=socket.recv().await {
                        sent.send(serde_json::from_str::<serde_json::Value>(&text).unwrap()).await.unwrap();
                    }
                })
            }
        }));
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address=listener.local_addr().unwrap();
        let server=tokio::spawn(async move {axum::serve(listener,app).await.unwrap()});
        let socket=protocol::fixture(address).await.unwrap();
        let (state,_,call)=super::super::tests::setup("grok_transcript_contract").await;
        let db=state.db.clone();
        let (input_tx,input)=tokio::sync::mpsc::channel(64);
        let (output,mut audio)=tokio::sync::mpsc::channel(64);
        let billing=crate::services::billing::BillingRouteContext::new(
            crate::services::billing::BillingIngress::LlmProvider,"fixture".into(),call.user_id.clone(),call.user_id.clone(),
            None,None,None,None,crate::services::billing::NodeIntent::Direct,"bearer".into(),
            crate::models::usage_meter::CredentialClass::UserOwned,crate::models::service_billing::BillingMetric::Tokens,None,false);
        let mut grok=Grok::new(socket,input,output,state.clone(),call.clone(),billing.clone());
        grok.arbiter.starting=true; grok.arbiter.created("speaking".into()).unwrap();
        grok.client(ClientInput::Begin).await.unwrap();
        grok.client(ClientInput::Pcm([32,16].repeat(2400))).await.unwrap_err(); // Oversized frames refused.
        for _ in 0..10 {grok.client(ClientInput::Pcm([32,16].repeat(480))).await.unwrap();}
        grok.client(ClientInput::Commit).await.unwrap();
        grok.provider(json!({"type":"input_audio_buffer.committed","item_id":"user1"})).await.unwrap();
        for (kind,text) in [("updated","Delete the wrong"),("updated","Delete the draft"),("completed","Delete the draft file")] {
            grok.provider(json!({"type":format!("conversation.item.input_audio_transcription.{kind}"),"item_id":"user1","transcript":text})).await.unwrap();
        }
        let segments:Vec<Segment>=grok.events.drain(..).map(|v|serde_json::from_value(v["segment"].clone()).unwrap()).collect();
        assert_eq!(segments.iter().map(|s|s.text.as_str()).collect::<Vec<_>>(),["Delete the wrong","Delete the draft","Delete the draft file"]);
        assert!(segments[2].sealed && segments[2].complete);
        assert_eq!(segments[0].id,segments[2].id);
        grok.provider(json!({"type":"conversation.item.input_audio_transcription.updated","item_id":"user1","transcript":"late rewrite"})).await.unwrap();
        assert!(grok.events.is_empty());
        grok.provider(json!({"type":"input_audio_buffer.speech_started"})).await.unwrap();
        assert_eq!(grok.arbiter.active.as_deref(),Some("speaking"));
        for _ in 0..11 {
            let event=tokio::time::timeout(std::time::Duration::from_secs(2),received.recv()).await.unwrap().unwrap();
            assert!(matches!(event["type"].as_str(),Some("input_audio_buffer.append"|"input_audio_buffer.commit")));
        }
        assert!(received.try_recv().is_err());
        // After a manual input is accepted, only its server-issued ID can delegate.
        grok.arbiter.done("speaking").unwrap();
        grok.send(json!({"type":"nyx.input.accepted","id":segments[2].id})).await.unwrap();
        grok.provider(json!({"type":"response.created","response":{"id":"answer"}})).await.unwrap();
        let function=json!({"type":"response.function_call_arguments.done","response_id":"answer","call_id":"tool1",
            "name":"delegate_to_agent","arguments":json!({"utterance_id":segments[2].id,"intent_hint":"delete draft"}).to_string()});
        grok.provider(function.clone()).await.unwrap();
        grok.provider(function).await.unwrap();
        let delegation=grok.events.pop_front().unwrap();
        assert_eq!(delegation["type"],"session.delegation.created");
        assert_eq!(delegation["utterance_id"],segments[2].id);
        assert!(grok.events.is_empty());
        grok.send(json!({"type":"nyx.task_receipt","call_id":"tool1","receipt":{"task_id":"durable-task","status":"queued"}})).await.unwrap();
        assert!(grok.arbiter.unresolved.is_empty());
        use base64::Engine;
        grok.provider(json!({"type":"response.audio.delta","response_id":"answer","item_id":"provider-item",
            "delta":base64::engine::general_purpose::STANDARD.encode([0;960])})).await.unwrap();
        assert!(matches!(audio.recv().await,Some(super::relay::Output::Audio{..})));
        grok.provider(json!({"type":"response.audio_transcript.done","response_id":"answer","item_id":"provider-item","transcript":"Delete the draft file — should I go ahead?"})).await.unwrap();
        grok.arbiter.cancelled.insert("answer".into());
        let done=json!({"type":"response.done","response":{"id":"answer","status":"completed","usage":{"input_tokens":20,"output_tokens":10,"total_tokens":30}}});
        grok.provider(done.clone()).await.unwrap();
        grok.provider(done).await.unwrap();
        let final_event=grok.events.back().unwrap();
        assert_eq!(final_event["segment"]["sealed"],true);
        assert_eq!(final_event["segment"]["complete"],false); // Cancelled speech cannot arm a readback.
        assert!(uuid::Uuid::parse_str(final_event["segment"]["id"].as_str().unwrap()).is_ok());
        assert_eq!(grok.events.len(),2); // Draft and one final, never a replay.
        // Trailing playback still has a truncation target after response.done.
        grok.events.clear();
        for (item, meaningful) in [("backchannel", false), ("correction", true)] {
            grok.client(ClientInput::Begin).await.unwrap();
            for _ in 0..16 { grok.client(ClientInput::Pcm([32,16].repeat(480))).await.unwrap(); }
            grok.client(ClientInput::Commit).await.unwrap();
            grok.provider(json!({"type":"input_audio_buffer.committed","item_id":item})).await.unwrap();
            grok.provider(json!({"type":"conversation.item.input_audio_transcription.completed","item_id":item,"transcript":"fixture utterance"})).await.unwrap();
            let segment: Segment=serde_json::from_value(grok.events.pop_front().unwrap()["segment"].clone()).unwrap();
            assert_eq!(grok.accept_input_with(&segment, &Gate(meaningful)).await.unwrap(),meaningful);
            if !meaningful { assert!(audio.try_recv().is_err()); }
        }
        assert!(matches!(audio.recv().await,Some(super::relay::Output::Flush)));
        let mut truncated=false;
        while let Ok(Some(event))=tokio::time::timeout(std::time::Duration::from_millis(50),received.recv()).await {
            if event["type"]=="conversation.item.truncate" {
                assert_eq!(event["item_id"],"provider-item");
                assert_eq!(event["audio_end_ms"],0);
                truncated=true;
            }
        }
        assert!(truncated);
        // End can arrive before the coordinator has read the actor's last
        // normalized caption. Close retains that caption for persistence only.
        grok.arbiter=Arbiter::default();
        grok.events.push_back(json!({"type":"nyx.transcript","segment":segments[2]}));
        // A browser disconnect races the coordinator's explicit End command.
        drop(input_tx);
        let mut handle=super::actor::Handle::spawn(grok);
        handle.close().await.unwrap();
        let pending=handle.take_pending();
        assert_eq!(pending.len(),1);
        assert_eq!(pending[0]["segment"]["id"],segments[2].id);
        assert!(handle.take_pending().is_empty());
        let closed=handle.measured_ms();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert_eq!(handle.measured_ms(),closed);
        db.drop().await.unwrap(); server.abort();
    }).await;
}

#[test]
fn grok_ack_refuses_missing_manual_mode_or_builtin_tools() {
    let config = protocol::configuration("catalog-model", "Eve", "context");
    let mut session = config["session"].clone();
    assert!(protocol::acknowledged(&session, "catalog-model", "Eve"));
    session.as_object_mut().unwrap().remove("turn_detection");
    assert!(!protocol::acknowledged(&session, "catalog-model", "Eve"));
    session = config["session"].clone();
    session["tools"]
        .as_array_mut()
        .unwrap()
        .push(json!({"type":"web_search"}));
    assert!(!protocol::acknowledged(&session, "catalog-model", "Eve"));
    session = config["session"].clone();
    session["resumption"]["enabled"] = true.into();
    assert!(!protocol::acknowledged(&session, "catalog-model", "Eve"));
    session = config["session"].clone();
    session["audio"]["output"]["format"]["rate"] = 48000.into();
    assert!(!protocol::acknowledged(&session, "catalog-model", "Eve"));
}

struct Gate(bool);
#[async_trait::async_trait]
impl super::gate::Classifier for Gate {
    async fn meaningful(&self, _text: &str) -> bool {
        self.0
    }
}

#[test]
fn grok_unfinished_captions_seal_incomplete_on_disconnect_not_on_silence() {
    use super::super::transcript::Transcripts;
    let mut transcripts = Transcripts::default();
    let id = uuid::Uuid::new_v4().to_string();
    let event = json!({"type":"nyx.transcript","segment":{"id":id,"speaker":"user","text":"unfinished action", "start_ms":0,"end_ms":100,"sealed":false,"complete":false}});
    transcripts.ingest(&event, 100).unwrap();
    assert!(transcripts.seal_ready(5000, false).is_empty());
    let tail = transcripts.seal_ready(5000, true);
    assert_eq!(tail.len(), 1);
    assert!(tail[0].sealed);
    assert!(!tail[0].complete);
    assert_eq!(tail[0].id, id);
    assert!(transcripts.seal_ready(5001, true).is_empty());
}

#[tokio::test]
async fn grok_start_rejections_preserve_status_or_safe_frame_identifiers() {
    use axum::{
        Json, Router,
        extract::{WebSocketUpgrade, ws::Message},
        http::StatusCode,
        response::IntoResponse,
        routing::get,
    };
    for handshake in [true, false] {
        let app = Router::new().route("/v1/realtime", get(move |ws: WebSocketUpgrade| async move {
            if handshake {
                return (StatusCode::UNAUTHORIZED, Json(json!({"error":{"type":"authentication_error","code":"invalid_api_key","message":"DO-NOT-EXPOSE"}}))).into_response();
            }
            ws.on_upgrade(|mut socket| async move {
                socket.send(Message::Text(json!({"type":"session.created"}).to_string().into())).await.unwrap();
                socket.recv().await.unwrap().unwrap();
                socket.send(Message::Text(json!({"type":"error","error":{"type":"invalid_request_error","code":"invalid_value","param":"session.voice","message":"DO-NOT-EXPOSE sk-secret instructions SDP"}}).to_string().into())).await.unwrap();
            })
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let error = protocol::fixture(address).await.err().unwrap();
        let body = error.response_body();
        let details = body.details.as_ref().unwrap();
        assert_eq!(error.error_code(), 12501);
        assert_eq!(details["stage"], "provider_create");
        assert_eq!(details["provider"], "xai");
        if handshake {
            assert_eq!(details["provider_status"], 401);
        } else {
            assert_eq!(details["provider_type"], "invalid_request_error");
            assert_eq!(
                details["reason"],
                "provider_create:invalid_value:session.voice"
            );
        }
        let rendered = serde_json::to_string(&body).unwrap() + &format!("{error:?}");
        assert!(!rendered.contains("DO-NOT-EXPOSE") && !rendered.contains("sk-secret"));
        server.abort();
    }
}

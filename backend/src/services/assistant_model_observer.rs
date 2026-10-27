//! Turn-fenced facts from the engine's HTTP model path. MCP inference tools use
//! their own transport and never enter this observer.
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use chrono::{DateTime, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
};
use uuid::Uuid;

use super::assistant_nyxagent::{self as engine, TurnError};
use crate::{
    errors::{AppError, AppResult},
    models::assistant_conversation::{ActiveTurn, AssistantConversation, COLLECTION_NAME},
};

use super::assistant_engine_policy::MODEL_ATTRIBUTION_WINDOW;

#[derive(Clone)]
pub struct ModelRequest {
    db: Database,
    conversation: String,
    turn: String,
    attempt: String,
    completion: Arc<Completion>,
}

struct Completion {
    db: Database,
    conversation: String,
    turn: String,
    done: AtomicBool,
}

impl Completion {
    async fn finish(&self) -> AppResult<()> {
        if self.done.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        complete_request(&self.db, &self.conversation, &self.turn).await?;
        Ok(())
    }
}

async fn complete_request(db: &Database, conversation: &str, turn: &str) -> AppResult<()> {
    db.collection::<bson::Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": conversation, "active_turn.turn_id": turn},
            vec![
                doc! {"$set": {"active_turn.model_requests_active": {"$max": [0, {"$subtract": [
                    {"$ifNull": ["$active_turn.model_requests_active", 0]}, 1
                ]}]}}},
            ],
        )
        .await?;
    Ok(())
}

impl Drop for Completion {
    fn drop(&mut self) {
        if !self.done.load(Ordering::SeqCst) {
            let db = self.db.clone();
            let conversation = self.conversation.clone();
            let turn = self.turn.clone();
            tokio::spawn(async move {
                let _ = complete_request(&db, &conversation, &turn).await;
            });
        }
    }
}

impl ModelRequest {
    pub async fn capture(
        db: &Database,
        conversation: &str,
        key: &str,
        turn: &str,
    ) -> AppResult<Option<Self>> {
        let row = db
            .collection::<AssistantConversation>(COLLECTION_NAME)
            .find_one(doc! {"_id": conversation, "credential_api_key_id": key, "active_turn.turn_id": turn})
            .await?;
        let captured = row
            .as_ref()
            .and_then(|row| engine::live_turn(row, Utc::now()))
            .filter(|turn| !turn.stop_requested)
            .map(|turn| Self {
                db: db.clone(),
                conversation: conversation.into(),
                turn: turn.turn_id.clone(),
                attempt: Uuid::new_v4().to_string(),
                completion: Arc::new(Completion {
                    db: db.clone(),
                    conversation: conversation.into(),
                    turn: turn.turn_id.clone(),
                    done: AtomicBool::new(true),
                }),
            });
        if let Some(request) = &captured {
            let matched = db
                .collection::<bson::Document>(COLLECTION_NAME)
                .update_one(
                    request.turn_fence(),
                    vec![doc! {"$set": {
                        "active_turn.model_attempt_id": &request.attempt,
                        "active_turn.model_attribution_uncertain": {"$or": [
                            {"$ifNull": ["$active_turn.model_attribution_uncertain", false]},
                            {"$gt": [{"$ifNull": ["$active_turn.model_requests_active", 0]}, 0]}
                        ]},
                        "active_turn.model_requests_active": {"$add": [
                            {"$ifNull": ["$active_turn.model_requests_active", 0]}, 1
                        ]}
                    }}],
                )
                .await?
                .matched_count;
            if matched == 0 {
                return Ok(None);
            }
            request.completion.done.store(false, Ordering::SeqCst);
        }
        Ok(captured)
    }

    fn turn_fence(&self) -> bson::Document {
        doc! {"_id": &self.conversation, "active_turn.turn_id": &self.turn,
        "active_turn.stop_requested": false}
    }

    fn fence(&self) -> bson::Document {
        let mut fence = self.turn_fence();
        fence.insert("active_turn.model_attempt_id", &self.attempt);
        fence
    }

    pub async fn refusal(&self, error: &AppError) -> AppResult<()> {
        if refusal_code(error).is_none() {
            return self.complete().await;
        }
        self.rejected(error).await
    }

    /// Only stable AppError metadata, never Display/debug strings or bodies.
    /// Called around the whole model proxy so pre-billing refusals are visible.
    pub async fn rejected(&self, error: &AppError) -> AppResult<()> {
        self.db
            .collection::<bson::Document>(COLLECTION_NAME)
            .update_one(
                self.fence(),
                doc! {"$set": {"active_turn.model_refusal": {
                    "code": error.error_key(), "http_status": i32::from(error.status_code().as_u16()),
                    "error_code": i64::from(error.error_code()),
                    "attempt_id": &self.attempt, "at": bson::DateTime::now()
                }}},
            )
            .await?;
        self.complete().await
    }

    pub async fn complete(&self) -> AppResult<()> {
        self.completion.finish().await
    }

    pub async fn accepted(&self) -> AppResult<()> {
        let mut fence = self.fence();
        fence.insert("active_turn.model_refusal", doc! {"$ne": null});
        self.db
            .collection::<bson::Document>(COLLECTION_NAME)
            .update_one(fence, doc! {"$unset": {"active_turn.model_refusal": ""}})
            .await?;
        Ok(())
    }

    pub async fn fault(&self, kind: &str, attributable: bool) -> AppResult<()> {
        let mut fence = self.fence();
        // Stop is still a live captured turn; its cancellation is diagnostic.
        fence.remove("active_turn.stop_requested");
        let field = if kind == "consumer_cancelled" {
            "active_turn.model_stream_cancellation"
        } else {
            "active_turn.model_stream_fault"
        };
        self.db.collection::<bson::Document>(COLLECTION_NAME).update_one(fence,
            doc! {"$set": {field: {
                "kind": kind, "attempt_id": &self.attempt, "at": bson::DateTime::now(),
                "attributable": attributable && kind != "consumer_cancelled" && kind != "unknown"
            }}}).await?;
        Ok(())
    }
}

pub fn refusal_code(error: &AppError) -> Option<&'static str> {
    billing_failure_code(error.error_key())
}

pub fn billing_failure_code(code: &str) -> Option<&'static str> {
    match code {
        "insufficient_credits" => Some("insufficient_credits"),
        "wallet_suspended" => Some("wallet_suspended"),
        "plan_entitlement_required" => Some("plan_entitlement_required"),
        "billing_not_configured" => Some("billing_not_configured"),
        "billing_provider_unavailable" => Some("billing_provider_unavailable"),
        _ => None,
    }
}

pub fn attribute(error: &TurnError, turn: &ActiveTurn, now: DateTime<Utc>) -> TurnError {
    let generic = (error.code == "assistant_unavailable"
        && error.upstream_code.is_some()
        && matches!(error.stage, "stream" | "upstream_response"))
        || (error.code == "invalid_stream" && error.stage == "stream");
    let billing_wrapper =
        error.code == "nyxid_error" && matches!(error.stage, "stream" | "upstream_response");
    if !(generic || billing_wrapper) || turn.model_attribution_uncertain {
        return error.clone();
    }
    let recent = |at: DateTime<Utc>| {
        at <= now
            && now.signed_duration_since(at)
                <= chrono::Duration::from_std(MODEL_ATTRIBUTION_WINDOW)
                    .expect("bounded attribution window")
    };
    let code = turn
        .model_refusal
        .as_ref()
        .filter(|fact| {
            turn.model_attempt_id.as_deref() == Some(&fact.attempt_id) && recent(fact.at)
        })
        .and_then(|fact| {
            billing_failure_code(&fact.code).or_else(|| {
                (fact.code == "guest_model_route_required"
                    && fact.http_status == Some(403)
                    && fact.error_code == Some(9000))
                .then_some("guest_model_route_required")
            })
        })
        .or_else(|| {
            turn.model_stream_fault
                .as_ref()
                .filter(|fact| {
                    generic
                        && turn.model_attempt_id.as_deref() == Some(&fact.attempt_id)
                        && fact.attributable
                        && recent(fact.at)
                        && matches!(
                            fact.kind.as_str(),
                            "transport_error" | "provider_error" | "missing_terminal"
                        )
                })
                .map(|_| "model_interrupted")
        });
    let Some(code) = code else {
        return error.clone();
    };
    let mut attributed = error.clone();
    let classified = TurnError::new(code);
    attributed.code = classified.code;
    attributed.message = classified.message;
    attributed
}

/// A recent, unambiguous failure of this turn's exact model attempt explains a
/// generic NyxAgent failure. Success clears it; old/parallel attempts never do.
pub fn failure_reason(
    error: &TurnError,
    turn: &ActiveTurn,
    now: DateTime<Utc>,
) -> Option<Box<crate::models::assistant_message::TurnFailureReason>> {
    use crate::models::assistant_message::TurnFailureReason;
    if matches!(error.stage, "stream" | "upstream_response")
        && matches!(
            error.upstream_code.as_deref(),
            Some("turn_failed" | "nyxid_error")
        )
        && !turn.model_attribution_uncertain
        && let Some(fact) = turn.model_refusal.as_ref().filter(|fact| {
            turn.model_attempt_id.as_deref() == Some(&fact.attempt_id)
                && fact.at <= now
                && now.signed_duration_since(fact.at)
                    <= chrono::Duration::from_std(MODEL_ATTRIBUTION_WINDOW).expect("bounded window")
                && fact.http_status.is_some()
                && fact.error_code.is_some()
        })
    {
        return Some(Box::new(TurnFailureReason {
            source: "nyxid".into(),
            stage: "model_proxy".into(),
            code: fact.code.clone(),
            http_status: fact.http_status,
            nyxid_error_code: fact.error_code,
        }));
    }
    error.upstream_code.as_ref().map(|code| {
        Box::new(TurnFailureReason {
            source: "nyxagent".into(),
            stage: error.stage.into(),
            code: code.to_string(),
            http_status: error.http_status,
            nyxid_error_code: None,
        })
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    Responses,
    ChatCompletions,
    Anthropic,
    Unknown,
}

pub fn protocol(path: &str) -> Protocol {
    let path = path.trim_matches('/');
    if path == "responses" || path.ends_with("/responses") {
        Protocol::Responses
    } else if path == "chat/completions" || path.ends_with("/chat/completions") {
        Protocol::ChatCompletions
    } else if path == "messages" || path.ends_with("/messages") {
        Protocol::Anthropic
    } else {
        Protocol::Unknown
    }
}

pub struct StreamObserver {
    protocol: Protocol,
    terminal: bool,
    failed: bool,
    decoder: super::sse_parser::BoundedEventDecoder,
    invalid_framing: bool,
}
impl StreamObserver {
    pub fn new(protocol: Protocol) -> Self {
        Self {
            protocol,
            terminal: false,
            failed: false,
            decoder: super::sse_parser::BoundedEventDecoder::with_max_bytes(
                super::assistant_engine_policy::MODEL_STREAM_OBSERVER_MAX_BUFFER_BYTES,
            ),
            invalid_framing: false,
        }
    }
    pub fn push_byte(&mut self, byte: u8) -> Option<super::sse_parser::SseEvent> {
        match self.decoder.push_byte(byte)? {
            Ok(event) => {
                self.event(event.event_type.as_deref(), &event.data);
                Some(event)
            }
            Err(_) => {
                self.invalidate();
                None
            }
        }
    }

    pub fn event(&mut self, event: Option<&str>, data: &str) {
        if self.protocol == Protocol::Unknown || self.invalid_framing {
            return;
        }
        if self.protocol == Protocol::ChatCompletions && data.trim() == "[DONE]" {
            self.terminal = true;
            return;
        }
        if data.is_empty() {
            return;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
            self.invalidate();
            return;
        };
        let kind = value["type"].as_str().or(event).unwrap_or_default();
        if kind == "error"
            || value.get("error").is_some_and(|v| !v.is_null())
            || matches!(kind, "response.failed" | "response.incomplete")
        {
            self.failed = true;
        }
        self.terminal |= match self.protocol {
            Protocol::Responses => matches!(
                kind,
                "response.completed" | "response.failed" | "response.incomplete"
            ),
            Protocol::ChatCompletions => data.trim() == "[DONE]",
            Protocol::Anthropic => kind == "message_stop",
            Protocol::Unknown => false,
        };
    }
    pub fn provider_failed(&self) -> bool {
        self.failed && !self.invalid_framing
    }
    pub fn invalidate(&mut self) {
        self.invalid_framing = true;
    }
    pub fn framing_invalid(&self) -> bool {
        self.invalid_framing
    }
    pub fn transport_fault(&self) -> (&'static str, bool) {
        if self.invalid_framing {
            ("unknown", false)
        } else {
            ("transport_error", true)
        }
    }
    pub fn eof(&self) -> Option<&'static str> {
        if self.invalid_framing || self.decoder.has_incomplete_event() {
            Some("unknown")
        } else if self.failed {
            Some("provider_error")
        } else if self.protocol == Protocol::Unknown {
            Some("unknown")
        } else if !self.terminal {
            Some("missing_terminal")
        } else {
            None
        }
    }
}

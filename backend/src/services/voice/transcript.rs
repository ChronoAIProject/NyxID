//! Authoritative sideband fragments. Browser captions never enter this buffer.
use crate::errors::{AppError, AppResult};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet, VecDeque};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Speaker {
    User,
    Assistant,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Segment {
    pub id: String,
    pub speaker: Speaker,
    pub text: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub sealed: bool,
    pub complete: bool,
}
impl std::fmt::Debug for Segment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Segment { [REDACTED] }")
    }
}

struct Fragment {
    end_ms: i64,
    text: String,
}
struct Pending {
    id: String,
    fragments: BTreeMap<(i64, String), Fragment>,
    last_received_ms: i64,
}
#[derive(Default)]
pub struct Transcripts {
    normalized_pending: BTreeMap<String, Segment>,
    input: Option<Pending>,
    output: Option<Pending>,
    seen: HashSet<String>,
    order: VecDeque<String>,
    sealed_input: i64,
    sealed_output: i64,
}

impl Transcripts {
    pub fn has_unsealed_user_input(&self) -> bool {
        self.input.is_some()
    }

    pub fn ingest(&mut self, event: &Value, now_ms: i64) -> AppResult<Vec<Segment>> {
        if event["type"] == "nyx.transcript" {
            let segment: Segment = serde_json::from_value(event["segment"].clone())
                .map_err(|_| AppError::VoiceProviderUnavailable)?;
            if segment.sealed {
                self.normalized_pending.remove(&segment.id);
            } else {
                self.normalized_pending
                    .insert(segment.id.clone(), segment.clone());
                if self.normalized_pending.len() > 8 {
                    return Err(AppError::VoiceProviderUnavailable);
                }
            }
            return Ok(vec![segment]);
        }
        let speaker = match event["type"].as_str() {
            Some("session.input_transcript.delta") => Speaker::User,
            Some("session.output_transcript.delta") => Speaker::Assistant,
            _ => return Ok(Vec::new()),
        };
        let invalid = || AppError::VoiceProviderUnavailable;
        let id = event["event_id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or_else(invalid)?;
        if self.seen.contains(id) {
            return Ok(Vec::new());
        }
        let start = event["start_ms"].as_i64().ok_or_else(invalid)?;
        let end = event["end_ms"].as_i64().ok_or_else(invalid)?;
        let text = event["delta"]
            .as_str()
            .filter(|s| s.len() <= 4096)
            .ok_or_else(invalid)?;
        if start < 0 || end < start || end > 1_830_000 {
            return Err(invalid());
        }
        self.seen.insert(id.into());
        self.order.push_back(id.into());
        if self.order.len() > 4096
            && let Some(old) = self.order.pop_front()
        {
            self.seen.remove(&old);
        }
        let (pending, sealed) = if speaker == Speaker::User {
            (&mut self.input, &mut self.sealed_input)
        } else {
            (&mut self.output, &mut self.sealed_output)
        };
        // Never rewrite a sealed/possibly executed utterance with a late fragment.
        if start < *sealed {
            return Ok(Vec::new());
        }
        let mut results = Vec::new();
        if let Some(p) = pending.as_ref() {
            let first = p.fragments.first_key_value().map_or(start, |(k, _)| k.0);
            let bytes: usize = p.fragments.values().map(|f| f.text.len()).sum();
            if end - first > 15_000 || bytes + text.len() > 4096 {
                let segment = assemble(p, speaker, true, false);
                *sealed = segment.end_ms;
                results.push(segment);
                *pending = None;
            }
        }
        let p = pending.get_or_insert_with(|| Pending {
            id: uuid::Uuid::new_v4().to_string(),
            fragments: BTreeMap::new(),
            last_received_ms: now_ms,
        });
        p.last_received_ms = now_ms;
        p.fragments.insert(
            (start, id.into()),
            Fragment {
                end_ms: end,
                text: text.into(),
            },
        );
        results.push(assemble(p, speaker, false, true));
        Ok(results)
    }

    pub fn seal_ready(&mut self, now_ms: i64, disconnected: bool) -> Vec<Segment> {
        let mut out = Vec::new();
        if disconnected {
            out.extend(
                std::mem::take(&mut self.normalized_pending)
                    .into_values()
                    .map(|mut s| {
                        s.sealed = true;
                        s.complete = false;
                        s
                    }),
            );
        }
        for (pending, sealed, speaker) in [
            (&mut self.input, &mut self.sealed_input, Speaker::User),
            (
                &mut self.output,
                &mut self.sealed_output,
                Speaker::Assistant,
            ),
        ] {
            if pending
                .as_ref()
                .is_some_and(|p| disconnected || now_ms - p.last_received_ms >= 1000)
            {
                let p = pending.take().expect("checked pending");
                let segment = assemble(&p, speaker, true, !disconnected);
                *sealed = segment.end_ms;
                out.push(segment);
            }
        }
        out
    }
}
fn assemble(p: &Pending, speaker: Speaker, sealed: bool, complete: bool) -> Segment {
    Segment {
        id: p.id.clone(),
        speaker,
        text: p.fragments.values().map(|f| f.text.as_str()).collect(),
        start_ms: p.fragments.first_key_value().map_or(0, |(k, _)| k.0),
        end_ms: p.fragments.values().map(|f| f.end_ms).max().unwrap_or(0),
        sealed,
        complete,
    }
}

/// Checkpoint a segment into the existing sequence allocator, under the session
/// lease. Transcript-only rows never become agent instructions by themselves.
pub async fn persist(
    db: &mongodb::Database,
    call: &crate::models::assistant_voice_session::VoiceSession,
    segment: &Segment,
) -> AppResult<()> {
    use crate::models::{
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as THREADS},
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES, VoiceTranscript},
        assistant_voice_session::COLLECTION_NAME as CALLS,
    };
    use mongodb::bson::{self, doc};
    let db = db.clone();
    let call = call.clone();
    let segment = segment.clone();
    let mut tx = db.client().start_session().await?;
    tx.start_transaction().and_run2(async move |tx| {
        let result:AppResult<()>=Box::pin(async {
            if db.collection::<bson::Document>(CALLS).update_one(super::session::fence(&call),doc!{"$set":{"transcript_fence":uuid::Uuid::new_v4().to_string()}})
                .session(&mut *tx).await?.matched_count!=1 {return Err(AppError::Conflict("Voice lease lost".into()));}
            let messages=db.collection::<AssistantMessage>(MESSAGES);
            let metadata=VoiceTranscript{session_id:call.id.clone(),segment_id:segment.id.clone(),start_ms:segment.start_ms,
                end_ms:segment.end_ms,sealed:segment.sealed,complete:segment.complete,delivery:if segment.speaker==Speaker::User {"received"}else{"generated"}.into(),
                request_id:None,backend_message_id:None};
            if let Some(row)=messages.find_one(doc!{"_id":&segment.id,"user_id":&call.user_id,"conversation_id":&call.conversation_id})
                .session(&mut *tx).await? {
                if row.voice.as_ref().is_some_and(|v|!v.sealed && v.request_id.is_none()) {
                    messages.update_one(doc!{"_id":&row.id},doc!{"$set":{"text":&segment.text,"voice":bson::to_bson(&metadata)
                        .map_err(|_|AppError::Internal("Voice transcript serialization failed".into()))?}}).session(&mut *tx).await?;
                }
                return Ok(());
            }
            let thread=db.collection::<AssistantConversation>(THREADS).find_one_and_update(
                doc!{"_id":&call.conversation_id,"user_id":&call.user_id},doc!{"$inc":{"message_count":1},"$set":{"updated_at":bson::DateTime::now()}})
                .return_document(mongodb::options::ReturnDocument::After).session(&mut *tx).await?
                .ok_or_else(||AppError::NotFound("Voice conversation unavailable".into()))?;
            messages.insert_one(AssistantMessage{id:segment.id.clone(),voice:Some(metadata),execution_pending:true,
                conversation_id:call.conversation_id.clone(),user_id:call.user_id.clone(),seq:thread.message_count,
                turn_id:uuid::Uuid::new_v4().to_string(),role:if segment.speaker==Speaker::User {"user"}else{"assistant"}.into(),
                text:segment.text.clone(),status:"completed".into(),error_code:None,created_at:chrono::Utc::now(),activities:Vec::new(),
                attachments:Vec::new(),origin:None,via:Some("voice".into())}).session(&mut *tx).await?;
            Ok(())
        }).await;
        super::super::api_key_mutation_service::transaction_result(result)
    }).await.map_err(super::super::api_key_mutation_service::map_transaction_error)
}

pub async fn delegate(
    db: &mongodb::Database,
    call: &crate::models::assistant_voice_session::VoiceSession,
    source_id: &str,
    segment: &Segment,
) -> AppResult<crate::models::assistant_voice::VoiceRequest> {
    use crate::models::assistant_voice::{RequestState, VoiceRequest};
    use mongodb::bson::doc;
    if !segment.sealed
        || !segment.complete
        || segment.speaker != Speaker::User
        || source_id.is_empty()
        || source_id.len() > 256
    {
        return Err(AppError::ValidationError(
            "Voice delegation needs a complete user segment".into(),
        ));
    }
    super::super::assistant_voice::require_enabled(db, &call.user_id).await?;
    super::super::assistant_voice::thread(db, &call.user_id, &call.conversation_id).await?;
    let request = VoiceRequest {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: call.user_id.clone(),
        conversation_id: call.conversation_id.clone(),
        task_conversation_id: None,
        session_id: call.id.clone(),
        source_id: source_id.into(),
        message_id: segment.id.clone(),
        message_seq: 0,
        turn_id: uuid::Uuid::new_v4().to_string(),
        state: RequestState::Queued,
        pending_acknowledgement_ids: Vec::new(),
        acknowledgement_id: None,
        credential_api_key_id: None,
        parent_turn_id: None,
        root_request_id: None,
        result_message_id: None,
        recovery_replays: 0,
        created_at: chrono::Utc::now(),
        expires_at: chrono::Utc::now() + chrono::Duration::minutes(30),
    };
    let db = db.clone();
    let call = call.clone();
    let text = segment.text.clone();
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .and_run2(async move |tx| {
            let result = Box::pin(async {
                if db
                    .collection::<mongodb::bson::Document>(
                        crate::models::assistant_voice_session::COLLECTION_NAME,
                    )
                    .update_one(
                        super::session::fence(&call),
                        doc! {"$set":{"admission_fence":uuid::Uuid::new_v4().to_string()}},
                    )
                    .session(&mut *tx)
                    .await?
                    .matched_count
                    != 1
                {
                    return Err(AppError::Conflict("Voice lease lost".into()));
                }
                super::super::assistant_voice::enqueue_in_session(&db, request.clone(), &text, tx)
                    .await
            })
            .await;
            super::super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::super::api_key_mutation_service::map_transaction_error)
}

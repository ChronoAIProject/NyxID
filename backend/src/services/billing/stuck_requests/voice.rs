//! Voice's evidence rule, shared by the read-only inventory and terminal decision.
use super::*;
use crate::models::{
    assistant_voice::{VoiceWindow, WINDOWS},
    assistant_voice_session::COLLECTION_NAME as SESSIONS,
    service_billing::PlatformUsage,
    voice_responses::ResponsesState,
};
use mongodb::{ClientSession, options::Hint};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Window(i64),
    Responses,
    Response,
}
impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Window(_) => "window",
            Self::Responses => "responses",
            Self::Response => "response",
        }
    }
}
pub(super) fn identity(request: &str) -> Option<(&str, Kind)> {
    let tail = request.strip_prefix("voice:")?;
    let (session, suffix) = tail.split_once(':')?;
    uuid::Uuid::parse_str(session).ok()?;
    let kind = if suffix == "responses" {
        Kind::Responses
    } else if let Some(n) = suffix.strip_prefix("response:") {
        if n.parse::<u64>().ok()? == 0 {
            return None;
        }
        Kind::Response
    } else {
        let n = suffix.parse::<i64>().ok()?;
        if !(0..1800).contains(&n) || n % 30 != 0 {
            return None;
        }
        Kind::Window(n)
    };
    Some((session, kind))
}
pub(super) fn is_voice(row: &Document) -> bool {
    // A configured ordinary price lane can include a voice_seconds component.
    // Only the request identity assigns it to the voice evidence protocol.
    row.get_str("billing_request_id")
        .ok()
        .is_some_and(|id| id.starts_with("voice:"))
}
pub(super) fn kind(row: &Document) -> &'static str {
    if !is_voice(row) {
        return "";
    }
    row.get_str("billing_request_id")
        .ok()
        .and_then(identity)
        .map_or("unknown", |(_, k)| k.label())
}
fn projection() -> Document {
    doc! {"_id":1,"user_id":1,"generation":1,"lease_owner":1,"lease_until":1,"live_slot":1,
    "responses_model":1,"responses_reserved":1,"responses_finalized":1,"responses_usage":1,
    "observed_seconds":1,"final_usage_confirmed":1,"reconcile_deadline":1,"provider_session_id":1}
}
/// A page without voice identities does no voice database work. Both readers
/// call this inside their existing snapshot transaction; identity indexes bound
/// the two optional batch reads to the distinct voice identities on that page.
pub(super) async fn read(
    db: &mongodb::Database,
    tx: &mut ClientSession,
    rows: &mut [Document],
) -> AppResult<()> {
    let mut sessions = BTreeSet::new();
    let mut windows = BTreeSet::new();
    for row in rows.iter() {
        if let Ok(id) = row.get_str("billing_request_id")
            && let Some((sid, k)) = identity(id)
        {
            sessions.insert(sid.to_owned());
            if matches!(k, Kind::Window(_)) {
                windows.insert(id.to_owned());
            }
        }
    }
    let mut calls = BTreeMap::new();
    if !sessions.is_empty() {
        let mut cursor = db
            .collection::<Document>(SESSIONS)
            .find(doc! {"_id":{"$in":sessions.into_iter().collect::<Vec<_>>()}})
            .projection(projection())
            .hint(Hint::Name("_id_".into()))
            .max_time(std::time::Duration::from_millis(500))
            .session(&mut *tx)
            .await?;
        while let Some(call) = next_document(&mut cursor, tx).await? {
            calls.insert(
                call.get_str("_id").map_err(|_| unavailable())?.to_owned(),
                call,
            );
        }
    }
    let mut checkpoints = BTreeMap::new();
    if !windows.is_empty() {
        let mut cursor = db
            .collection::<Document>(WINDOWS)
            .find(doc! {"billing_request_id":{"$in":windows.into_iter().collect::<Vec<_>>()}})
            .hint(Hint::Name("billing_request_id_1".into()))
            .max_time(std::time::Duration::from_millis(500))
            .session(&mut *tx)
            .await?;
        while let Some(w) = next_document(&mut cursor, tx).await? {
            checkpoints.insert(
                w.get_str("billing_request_id")
                    .map_err(|_| unavailable())?
                    .to_owned(),
                w,
            );
        }
    }
    for row in rows {
        if let Ok(id) = row.get_str("billing_request_id")
            && let Some((sid, _)) = identity(id)
        {
            let call = calls.get(sid).cloned();
            let window = checkpoints.get(id).cloned();
            row.insert("voice_session", call.map_or(Bson::Null, Bson::Document));
            row.insert("voice_window", window.map_or(Bson::Null, Bson::Document));
        }
    }
    Ok(())
}
#[derive(serde::Deserialize)]
struct Session {
    #[serde(rename = "_id")]
    id: String,
    user_id: String,
    generation: i64,
    lease_owner: String,
    lease_until: bson::DateTime,
    live_slot: bool,
    observed_seconds: i64,
    final_usage_confirmed: bool,
    reconcile_deadline: bson::DateTime,
    provider_session_id: Option<String>,
    #[serde(flatten)]
    responses: ResponsesState,
}
pub(super) struct Evidence {
    pub usage: Option<PlatformUsage>,
    pub model: Option<String>,
    session: Option<(String, i64)>,
    window: Option<String>,
}
pub(super) enum Decision {
    Ready(Box<Evidence>),
    Deferred(&'static str),
}
fn optional<T: serde::de::DeserializeOwned>(row: &Document, key: &str) -> Result<Option<T>, ()> {
    match row.get(key) {
        None | Some(Bson::Null) => Ok(None),
        Some(Bson::Document(value)) => bson::from_document(value.clone()).map(Some).map_err(|_| ()),
        _ => Err(()),
    }
}
pub(super) fn decide(row: &Document, registry: &Registry) -> Decision {
    decide_checked(row, registry).unwrap_or(Decision::Deferred("voice_malformed_evidence"))
}
fn decide_checked(row: &Document, registry: &Registry) -> Result<Decision, ()> {
    let request = row.get_str("billing_request_id").map_err(|_| ())?;
    let Some((sid, kind)) = identity(request) else {
        return Ok(Decision::Deferred("excluded_voice_unknown_shape"));
    };
    let call: Option<Session> = optional(row, "voice_session")?;
    let window: Option<VoiceWindow> = optional(row, "voice_window")?;
    let mut actors = vec![row.get_str("actor_user_id").map_err(|_| ())?];
    if let Ok(siblings) = row.get_array("request_rows") {
        for sibling in siblings {
            actors.push(
                sibling
                    .as_document()
                    .ok_or(())?
                    .get_str("actor_user_id")
                    .map_err(|_| ())?,
            );
        }
    }
    if actors.iter().any(|a| a.is_empty()) {
        return Err(());
    }
    if let Some(c) = &call {
        if c.id != sid
            || c.user_id.is_empty()
            || c.lease_owner.is_empty()
            || !(0..i64::MAX).contains(&c.generation)
            || !(0..=1800).contains(&c.observed_seconds)
        {
            return Err(());
        }
        if actors.iter().any(|a| *a != c.user_id) {
            return Ok(Decision::Deferred("voice_actor_mismatch"));
        }
        if registry.generation_is_live(&c.lease_owner)
            && (c.live_slot || c.lease_until > registry.now)
        {
            return Ok(Decision::Deferred("voice_live_session"));
        }
        if !matches!(kind, Kind::Response)
            && c.provider_session_id.is_some()
            && !c.final_usage_confirmed
            && c.reconcile_deadline > registry.now
        {
            return Ok(Decision::Deferred("voice_evidence_pending"));
        }
    }
    let mut evidence = Evidence {
        usage: None,
        model: None,
        session: call.as_ref().map(|c| (c.id.clone(), c.generation)),
        window: None,
    };
    match kind {
        Kind::Responses => {
            if let Some(call) = &call
                && let Some(reports) = call.responses.usage()
            {
                // checkpoint_usage refuses the next distinct report at this same
                // writer-owned bound. It never creates a larger legitimate map.
                if reports.len() > crate::services::voice::RESPONSE_USAGE_LIMIT {
                    return Err(());
                }
                if !reports.is_empty() {
                    evidence.usage = Some(crate::services::voice::recorded_response_usage(
                        &call.responses,
                    ));
                    evidence.model = call.responses.model().map(str::to_owned);
                }
            }
        }
        Kind::Response => {}
        Kind::Window(start) => {
            let seconds = if let Some(w) = window {
                if w.id.is_empty()
                    || w.billing_request_id != request
                    || w.session_id != sid
                    || w.start_second != start
                    || w.reserved_seconds != 30
                    || !(0..=30).contains(&w.observed_seconds)
                {
                    return Err(());
                }
                if actors.iter().any(|a| *a != w.user_id) {
                    return Ok(Decision::Deferred("voice_actor_mismatch"));
                }
                if !w.sealed
                    && bson::DateTime::from_chrono(w.deadline) > registry.now
                    && !call.as_ref().is_some_and(|c| c.final_usage_confirmed)
                {
                    return Ok(Decision::Deferred("voice_evidence_pending"));
                }
                evidence.window = Some(w.id);
                // Existing finalize() uses this exact checkpoint. Do not raise
                // it from session evidence: either race order must charge alike.
                Some(w.observed_seconds)
            } else {
                call.as_ref()
                    .map(|c| (c.observed_seconds - start).clamp(0, 30))
            };
            if let Some(seconds) = seconds {
                evidence.usage = Some(PlatformUsage {
                    voice_seconds: seconds,
                    requests: i64::from(start == 0 && seconds > 0),
                    ..Default::default()
                });
            }
        }
    }
    Ok(Decision::Ready(Box::new(evidence)))
}
impl Evidence {
    /// Called only after a nonempty complete publication decision. Errors abort
    /// all writes; the caller has no non-publication return after this boundary.
    pub(super) async fn fence(
        &self,
        db: &mongodb::Database,
        tx: &mut ClientSession,
    ) -> AppResult<()> {
        if let Some(id) = &self.window {
            let result = db
                .collection::<Document>(WINDOWS)
                .update_one(
                    doc! {"_id":id},
                    // Preserve already-sealed evidence, but always make a real
                    // write so a concurrent finalizer conflicts with this view.
                    vec![doc! {"$set":{
                        "uncertain":{"$cond":[{"$eq":["$sealed",true]},"$uncertain",true]},
                        "sealed":true,
                        "recovery_fence":uuid::Uuid::new_v4().to_string(),
                    }}],
                )
                .session(&mut *tx)
                .await?;
            if result.matched_count != 1 {
                return Err(unavailable());
            }
        }
        if let Some((id, generation)) = &self.session {
            let result = db
                .collection::<Document>(SESSIONS)
                .update_one(
                    doc! {"_id":id,"generation":generation},
                    doc! {"$inc":{"generation":1_i64}},
                )
                .session(&mut *tx)
                .await?;
            if result.matched_count != 1 {
                return Err(unavailable());
            }
        }
        Ok(())
    }
}

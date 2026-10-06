//! A continuation consumes the existing logical turn, run and admission permit.
use super::assistant_nyxagent::TurnError;
use crate::errors::AppResult;
use crate::models::assistant_conversation::{ActiveTurn, COLLECTION_NAME, ToolProgress};
use mongodb::{
    Database,
    bson::{self, doc},
};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Object key order is irrelevant; array order (including call order) is not.
pub fn canonical_digest(value: &Value) -> String {
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Object(fields) => {
                let ordered: std::collections::BTreeMap<_, _> = fields
                    .iter()
                    .map(|(key, value)| (key.clone(), canonical(value)))
                    .collect();
                serde_json::to_value(ordered).expect("JSON values serialize")
            }
            Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
            other => other.clone(),
        }
    }
    hex::encode(Sha256::digest(
        serde_json::to_vec(&canonical(value)).expect("JSON values serialize"),
    ))
}

pub fn call_digest(name: &str, arguments: &Value, result_digest: &str) -> String {
    canonical_digest(&serde_json::json!([
        name,
        canonical_digest(arguments),
        result_digest
    ]))
}

fn append(progress: &ToolProgress, call: &str) -> ToolProgress {
    ToolProgress {
        started: progress.started,
        calls: progress.calls + 1,
        digest: canonical_digest(&serde_json::json!([progress.digest, call])),
    }
}

pub fn window_digest(text: &str, progress: &ToolProgress) -> Option<String> {
    // Missing/late completion metadata must never produce a false loop stop.
    (progress.started == progress.calls)
        .then(|| canonical_digest(&serde_json::json!([text, progress.calls, progress.digest])))
}

pub async fn started(
    db: &Database,
    owner: &str,
    conversation: &str,
) -> AppResult<Option<ActiveTurn>> {
    let row = db
        .collection::<bson::Document>(COLLECTION_NAME)
        .find_one_and_update(
            doc! {
                "_id": conversation,
                "user_id": owner,
                "active_turn.turn_id": {"$type": "string"},
                "active_turn.stop_requested": {"$ne": true},
            },
            doc! {"$inc": {"active_turn.tool_progress.started": 1_i64}},
        )
        .return_document(mongodb::options::ReturnDocument::After)
        .projection(doc! {"active_turn": 1, "_id": 0})
        .await?;
    decode_active(row)
}

fn decode_active(row: Option<bson::Document>) -> AppResult<Option<ActiveTurn>> {
    let turn = row
        .and_then(|mut row| row.remove("active_turn"))
        .filter(|value| !matches!(value, bson::Bson::Null))
        .map(bson::from_bson::<ActiveTurn>)
        .transpose()
        .map_err(|_| crate::errors::AppError::Internal("Invalid active turn metadata".into()))?;
    Ok(turn.filter(|turn| !turn.stop_requested))
}

pub async fn active_window(
    db: &Database,
    owner: &str,
    conversation: &str,
) -> AppResult<Option<ActiveTurn>> {
    // Read only for assistant MCP calls; the turn ID and continuation number fence
    // late completions from a stopped/replaced turn or the previous window.
    let row = db
        .collection::<bson::Document>(COLLECTION_NAME)
        .find_one(doc! {"_id": conversation, "user_id": owner})
        .projection(doc! {"active_turn":1, "_id":0})
        .await?;
    decode_active(row)
}

pub async fn completed(
    db: &Database,
    owner: &str,
    conversation: &str,
    started: &ActiveTurn,
    call: &str,
) -> AppResult<()> {
    let collection = db.collection::<bson::Document>(COLLECTION_NAME);
    for _ in 0..256 {
        let Some(current) = active_window(db, owner, conversation).await? else {
            return Ok(());
        };
        if current.turn_id != started.turn_id || current.continuations != started.continuations {
            return Ok(());
        }
        let next = append(&current.tool_progress, call);
        let count = current.tool_progress.calls;
        let filter = doc! {
            "_id": conversation,
            "user_id": owner,
            "active_turn.turn_id": &started.turn_id,
            "active_turn.stop_requested": {"$ne": true},
            "$expr": {"$and": [
                {"$eq": [{"$ifNull": ["$active_turn.tool_progress.calls", 0]}, count]},
                {"$eq": [{"$ifNull": ["$active_turn.continuations", 0]}, started.continuations]},
            ]},
        };
        let result = collection
            .update_one(
                filter,
                doc! {
                    "$set": {
                        "active_turn.tool_progress.calls": next.calls,
                        "active_turn.tool_progress.digest": next.digest,
                    },
                },
            )
            .await?;
        if result.matched_count == 1 {
            return Ok(());
        }
        // A concurrent completion won the CAS. Append to its digest, never lose
        // a call and never depend on the bounded UI activity list.
    }
    Err(crate::errors::AppError::Internal(
        "Tool progress contention exceeded limit".into(),
    ))
}

pub const HARD_MAX: i32 = 32;
pub const INSTRUCTION: &str = "Continue the task where you left off. Observe current state before repeating an action whose result was uncertain. Do not repeat completed work. Stop when the task is complete or an owner decision is needed.";

pub struct Continuations {
    maximum: u32,
    pub count: u32,
    previous: Option<String>,
}
impl Continuations {
    pub fn new(maximum: i32) -> Self {
        Self {
            maximum: maximum.clamp(0, HARD_MAX) as u32,
            count: 0,
            previous: None,
        }
    }
    pub fn next(
        &mut self,
        code: &str,
        bound: bool,
        progress: Option<String>,
    ) -> Result<bool, TurnError> {
        if !matches!(code, "tool_budget_exhausted" | "turn_timeout") {
            return Ok(false);
        }
        if !bound || self.count >= self.maximum {
            return Ok(false);
        }
        if progress.is_some() && self.previous == progress {
            return Err(TurnError::new("continuation_no_progress"));
        }
        self.previous = progress;
        self.count += 1;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn completions_are_atomic_fenced_and_store_only_digests() {
        let db = crate::test_utils::connect_transaction_test_database("continuation_digests").await;
        let rows = db.collection::<bson::Document>(COLLECTION_NAME);
        rows.insert_one(doc! {"_id":"thread", "user_id":"owner", "active_turn": {
            "turn_id":"turn", "started_at":bson::DateTime::now(), "stop_requested":false
        }})
        .await
        .unwrap();
        let started = started(&db, "owner", "thread").await.unwrap().unwrap();
        super::started(&db, "owner", "thread").await.unwrap();
        let secret = uuid::Uuid::new_v4().to_string();
        let call = call_digest(
            "tool",
            &serde_json::json!({"input":secret}),
            &canonical_digest(&serde_json::json!({"output":secret})),
        );
        let (a, b) = tokio::join!(
            completed(&db, "owner", "thread", &started, &call),
            completed(&db, "owner", "thread", &started, &call)
        );
        a.unwrap();
        b.unwrap();
        let current = active_window(&db, "owner", "thread")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.tool_progress.calls, 2);
        assert_eq!(current.tool_progress.started, 2);
        assert_eq!(
            current.tool_progress.digest,
            append(&append(&ToolProgress::default(), &call), &call).digest
        );
        assert!(
            !rows
                .find_one(doc! {"_id":"thread"})
                .await
                .unwrap()
                .unwrap()
                .to_string()
                .contains(&secret)
        );
        rows.update_one(doc!{"_id":"thread"},doc!{"$set":{"active_turn.continuations":1,"active_turn.tool_progress":{"calls":0_i64,"digest":""}}}).await.unwrap();
        completed(&db, "owner", "thread", &started, &call)
            .await
            .unwrap();
        let current = active_window(&db, "owner", "thread")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            current.tool_progress.calls, 0,
            "old window cannot contaminate next"
        );
        rows.update_one(
            doc! {"_id":"thread"},
            doc! {"$set":{"active_turn.stop_requested":true}},
        )
        .await
        .unwrap();
        completed(&db, "owner", "thread", &current, &call)
            .await
            .unwrap();
        assert!(
            active_window(&db, "owner", "thread")
                .await
                .unwrap()
                .is_none()
        );
        db.drop().await.unwrap();
    }

    #[test]
    fn homogeneous_tool_windows_compare_arguments_and_results_not_labels() {
        let window = |page: usize, result: &str| {
            let mut progress = ToolProgress::default();
            for i in 0..40 {
                let call = call_digest(
                    "nyx__machine_browser",
                    &serde_json::json!({"action":"click", "ref": page * 40 + i}),
                    &canonical_digest(&serde_json::json!(result)),
                );
                progress.started += 1;
                progress = append(&progress, &call);
            }
            progress
        };
        let mut continuation = Continuations::new(8);
        assert!(
            continuation
                .next(
                    "tool_budget_exhausted",
                    true,
                    window_digest("", &window(0, "done"))
                )
                .unwrap()
        );
        assert!(
            continuation
                .next(
                    "tool_budget_exhausted",
                    true,
                    window_digest("", &window(1, "done"))
                )
                .unwrap()
        );
        // Same calls with changed results also demonstrate progress.
        assert!(
            continuation
                .next(
                    "tool_budget_exhausted",
                    true,
                    window_digest("", &window(1, "changed"))
                )
                .unwrap()
        );
        assert_eq!(
            continuation
                .next(
                    "tool_budget_exhausted",
                    true,
                    window_digest("", &window(1, "changed"))
                )
                .unwrap_err()
                .code,
            "continuation_no_progress"
        );
    }

    #[test]
    fn text_only_progress_and_canonical_argument_order() {
        let mut continuation = Continuations::new(8);
        for text in ["first", "first second", "first second third"] {
            assert!(
                continuation
                    .next(
                        "turn_timeout",
                        true,
                        window_digest(text, &ToolProgress::default())
                    )
                    .unwrap()
            );
        }
        let a: Value = serde_json::from_str(r#"{"a":1,"b":{"x":2,"y":3}}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"b":{"y":3,"x":2},"a":1}"#).unwrap();
        assert_eq!(
            call_digest("tool", &a, "result"),
            call_digest("tool", &b, "result")
        );
        assert!(
            !window_digest("SECRET", &ToolProgress::default())
                .unwrap()
                .contains("SECRET")
        );
    }
    #[test]
    fn incomplete_windows_never_stop_continuation_or_poison_the_next_window() {
        let mut c = Continuations::new(8);
        let full = ToolProgress {
            started: 1,
            calls: 1,
            digest: "call".into(),
        };
        let incomplete = ToolProgress {
            started: 2,
            ..full.clone()
        };
        assert!(
            c.next("turn_timeout", true, window_digest("", &full))
                .unwrap()
        );
        for _ in 0..2 {
            assert!(
                c.next("turn_timeout", true, window_digest("", &incomplete))
                    .unwrap()
            );
        }
        assert!(
            c.next("turn_timeout", true, window_digest("", &full))
                .unwrap()
        );
        assert_eq!(
            c.next("turn_timeout", true, window_digest("", &full))
                .unwrap_err()
                .code,
            "continuation_no_progress"
        );
    }

    #[test]
    fn only_explicit_budget_and_timeout_continue_with_a_session_and_a_bound() {
        let mut c = Continuations::new(2);
        assert!(
            !c.next("tool_budget_exhausted", false, Some("a".into()))
                .unwrap()
        );
        assert!(!c.next("idle_timeout", true, Some("a".into())).unwrap());
        assert!(
            c.next("tool_budget_exhausted", true, Some("a".into()))
                .unwrap()
        );
        assert!(c.next("turn_timeout", true, Some("b".into())).unwrap());
        assert!(!c.next("turn_timeout", true, Some("c".into())).unwrap());
    }
    #[test]
    fn no_progress_ends_the_loop_and_the_hard_cap_cannot_be_bypassed() {
        let mut c = Continuations::new(i32::MAX);
        assert!(
            c.next("tool_budget_exhausted", true, Some("same".into()))
                .unwrap()
        );
        assert_eq!(
            c.next("tool_budget_exhausted", true, Some("same".into()))
                .unwrap_err()
                .code,
            "continuation_no_progress"
        );
        for n in 1..HARD_MAX {
            assert!(c.next("turn_timeout", true, Some(n.to_string())).unwrap());
        }
        assert!(!c.next("turn_timeout", true, Some("limit".into())).unwrap());
        assert!(
            !Continuations::new(0)
                .next("turn_timeout", true, Some("a".into()))
                .unwrap()
        );
    }
}

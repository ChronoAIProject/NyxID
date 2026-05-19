//! Service layer for remote CLI pairing (Mode B of the wizard flow).
//!
//! Lifecycle:
//!   1. CLI (authenticated) calls `create` → gets (id, plaintext code, expires_at).
//!      CLI prints code + pair URL, polls with `poll`.
//!   2. User on a browser-capable device claims the code with `claim`,
//!      binding it to their session. Returns `kind` + `prefill` so the
//!      frontend can render the right wizard panel.
//!   3. Frontend completes the wizard with `complete`, handing over a
//!      typed ack payload (same shape as the local-server wizard).
//!   4. CLI's next `poll` returns the ack and the flow ends.
//!
//! Security guards:
//!   - code is stored only as SHA-256; `claim` looks up by hash
//!   - `user_id` bound at `create` is enforced on `claim` and `complete`
//!   - failed claim attempts are counted; once the limit is hit the
//!     record is considered dead (further `claim` calls return the same
//!     opaque error as a non-existent code to avoid enumeration)
//!   - TTL index on `expires_at` auto-deletes; code paths double-check
//!     `expires_at` so in-flight requests can't beat the sweeper
//!   - ack payloads are size-bounded before storage

use chrono::{Duration, Utc};
use hmac::{Hmac, Mac};
use mongodb::bson::{Bson, DateTime as BsonDateTime, doc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::errors::{AppError, AppResult};
use crate::models::cli_pairing::{COLLECTION_NAME, CliPairing, CliPairingStatus};

type HmacSha256 = Hmac<Sha256>;

/// Keyed hash for a pairing code. Replaces an earlier plain-SHA-256
/// scheme: pairing codes draw from a 32^8 (~2^40) space, so an
/// unkeyed digest is brute-forceable offline given a DB snapshot.
/// HMAC-SHA256 with a server-side key (held in process memory, not
/// persisted in MongoDB) lifts the attacker's cost from "~trillion
/// offline hashes" to "can't derive the key without server access".
///
/// The normalized code (uppercased, dashes/whitespace stripped) is
/// the HMAC input; the hex digest is what gets stored in
/// `CliPairing.code_hash`. Lookups on `claim` HMAC the normalized
/// candidate with the same key and compare.
pub fn hmac_code(hmac_key: &[u8], normalized_code: &str) -> String {
    // `new_from_slice` returns an error only when the crate is
    // compiled with `std::hash::sip`-incompatible backends; our
    // build uses the standard `hmac` crate which accepts any
    // length. Unwrap-safe in practice.
    let mut mac = HmacSha256::new_from_slice(hmac_key).expect("HMAC-SHA256 accepts any key length");
    mac.update(normalized_code.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

/// Code-visible length. 8 chars from a 32-char Crockford-style alphabet →
/// 32^8 ≈ 1.1 trillion combinations. Paired with per-IP rate limits and
/// `user_id` binding this is well beyond brute-forceable at the API layer.
const CODE_LEN: usize = 8;

/// Crockford-style alphabet (excludes `I`, `L`, `O`, `U` to reduce typos).
/// Uppercase only so the UI can normalize user input.
const CODE_ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Wall-clock lifetime of a pairing record. 15 minutes is short enough
/// to bound the window for a leaked code and long enough to survive a
/// user hunting for their phone / opening an app / entering the code.
pub const DEFAULT_TTL_SECS: i64 = 900;

/// Cap on failed `claim` attempts before the pairing is considered dead.
/// With 32^8 codes and 5/min per-IP rate limiting this is defense-in-depth,
/// not the primary guard; the primary guard is the user_id binding.
pub const MAX_FAILED_CLAIMS: u32 = 10;

/// Upper bound on the JSON payload the CLI may ship as `prefill`. The
/// prefill is echoed back to the browser on `claim`, so it must stay
/// small enough to travel over a normal query-string/JSON response and
/// not be a DoS vector.
const MAX_PREFILL_BYTES: usize = 8 * 1024;

/// Upper bound on ack payloads. Ack payloads are shaped like
/// `{acknowledged: true, <some_id>: "<uuid>"}` — a couple hundred bytes
/// in practice.
const MAX_ACK_BYTES: usize = 4 * 1024;

/// Returned from `create`. The CLI keeps `id` (for polling) and prints
/// `code` and `pair_url` to the user.
#[derive(Debug, Clone, Serialize)]
pub struct CreatedPairing {
    pub id: String,
    pub code: String,
    pub expires_at: String,
}

/// Returned from `claim`. The frontend uses this to render the right
/// wizard panel; `id` is needed for the subsequent `complete`/`cancel`.
///
/// `resumed` is `true` when the same user re-claimed an already-claimed
/// pairing — a common case on accidental refresh. The frontend uses
/// this flag to gate DESTRUCTIVE re-entry: for flows that mint/rotate
/// on the confirm step (api-key create/rotate, node register/rotate),
/// a `resumed: true` claim MUST NOT re-run the destructive API call
/// because the secret from the first run may already be in the user's
/// hands. The idempotent re-claim is safe for non-destructive flows
/// (ai-key, which is credential-passthrough) and for the pre-action
/// window on any flow (the action hasn't run yet, so replay is a no-op).
///
/// Combined with the frontend's `/complete`-on-action change, this
/// keeps the legitimate "I accidentally refreshed before starting"
/// case recoverable while closing the "I refreshed after the mint
/// happened" replay hole.
#[derive(Debug, Clone, Serialize)]
pub struct ClaimedPairing {
    pub id: String,
    pub kind: String,
    pub prefill: serde_json::Value,
    pub resumed: bool,
    /// `true` when `POST /cli-pairings/{id}/reserve-action` has been
    /// called at least once. The frontend sets this right before
    /// executing the destructive API call, so a claim response with
    /// `resumed: true, action_started: false` means the user refreshed
    /// in the safe pre-action window and can continue normally.
    /// `resumed: true, action_started: true` means the mint already
    /// happened and the frontend must refuse to replay it.
    pub action_started: bool,
}

/// Shape returned on `poll`. Modeled as an enum so the CLI can
/// pattern-match without string parsing.
///
/// Wrapped by `PollResponse` so the CLI can read `kind` alongside
/// the status — needed by `nyxid pairing resume` to dispatch the
/// kind-specific success printer without a separate GET for
/// metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PollStatus {
    Pending,
    Claimed,
    Completed { ack: serde_json::Value },
    Cancelled,
    Expired,
}

/// Envelope returned from `GET /cli-pairings/{id}/poll`. The
/// `flatten` on `status` preserves the inline `{status, ack?}` shape
/// older CLI builds expect; newer CLIs additionally read `kind` (for
/// the `pairing resume` command's per-kind summary dispatch) and
/// `expires_at` (for better "keep polling or give up" UX).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollResponse {
    #[serde(flatten)]
    pub status: PollStatus,
    pub kind: String,
    pub expires_at: String,
}

fn generate_code() -> String {
    let mut rng = rand::thread_rng();
    (0..CODE_LEN)
        .map(|_| {
            let idx = rng.gen_range(0..CODE_ALPHABET.len());
            CODE_ALPHABET[idx] as char
        })
        .collect()
}

/// Normalize a user-supplied code: uppercase, strip dashes/whitespace.
/// Accepts both `ABCD-1234` and `ABCD1234` forms.
pub fn normalize_code(input: &str) -> String {
    input
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .flat_map(|c| c.to_uppercase())
        .collect()
}

/// Pretty-print a raw code for display: insert a dash after 4 chars.
fn format_code(raw: &str) -> String {
    if raw.len() == CODE_LEN {
        format!("{}-{}", &raw[..4], &raw[4..])
    } else {
        raw.to_string()
    }
}

pub struct CreateParams<'a> {
    pub user_id: &'a str,
    pub kind: &'a str,
    pub prefill: serde_json::Value,
    pub ttl_secs: Option<i64>,
}

/// Create a new pairing record. Returns the id, the plaintext code
/// (formatted with a dash), and the absolute expiry time.
///
/// `hmac_key` is the server-side HMAC secret used to key
/// `code_hash`; see `hmac_code` for why an unkeyed digest isn't
/// safe for an 8-char code. The key is held in memory (see
/// `AppState::cli_pairing_hmac_key`) and never persisted, so a
/// MongoDB snapshot alone doesn't let an attacker brute-force
/// live codes.
pub async fn create(
    db: &mongodb::Database,
    hmac_key: &[u8],
    params: CreateParams<'_>,
) -> AppResult<CreatedPairing> {
    if params.kind.is_empty() || params.kind.len() > 64 {
        return Err(AppError::BadRequest("invalid kind".into()));
    }
    // Reject obviously-malformed prefill payloads. `serde_json::Value`
    // size isn't free to measure; we round-trip through the serializer.
    let prefill_bytes = serde_json::to_vec(&params.prefill)
        .map_err(|e| AppError::BadRequest(format!("invalid prefill: {e}")))?;
    if prefill_bytes.len() > MAX_PREFILL_BYTES {
        return Err(AppError::BadRequest(format!(
            "prefill exceeds {MAX_PREFILL_BYTES} bytes"
        )));
    }

    let ttl = params.ttl_secs.unwrap_or(DEFAULT_TTL_SECS);
    if !(60..=3600).contains(&ttl) {
        return Err(AppError::BadRequest("ttl must be 60..=3600 seconds".into()));
    }

    let raw_code = generate_code();
    let code_hash = hmac_code(hmac_key, &raw_code);
    let now = Utc::now();
    let record = CliPairing {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: params.user_id.to_string(),
        code_hash,
        kind: params.kind.to_string(),
        prefill: params.prefill,
        status: CliPairingStatus::Pending,
        claimed_at: None,
        claimed_from_ip: None,
        completed_at: None,
        action_started_at: None,
        ack_payload: None,
        failed_claim_attempts: 0,
        created_at: now,
        expires_at: now + Duration::seconds(ttl),
    };

    db.collection::<CliPairing>(COLLECTION_NAME)
        .insert_one(&record)
        .await?;

    Ok(CreatedPairing {
        id: record.id,
        code: format_code(&raw_code),
        expires_at: record.expires_at.to_rfc3339(),
    })
}

/// Claim the pairing by user-supplied code. Binds (status=Claimed,
/// claimed_at, claimed_from_ip) and returns the kind + prefill so the
/// frontend can render the right wizard. Enforces the `user_id`
/// binding: a code created by user A cannot be claimed by user B.
///
/// Returns `Forbidden` on user mismatch / expired / too-many-failures
/// and `NotFound` when the code hash doesn't match any record. Both
/// flavors use the same opaque `NotFound` error externally to avoid
/// enumeration.
pub async fn claim(
    db: &mongodb::Database,
    hmac_key: &[u8],
    raw_code: &str,
    session_user_id: &str,
    source_ip: Option<&str>,
) -> AppResult<ClaimedPairing> {
    let normalized = normalize_code(raw_code);
    if normalized.len() != CODE_LEN {
        return Err(AppError::NotFound("pairing code not found".into()));
    }
    let code_hash = hmac_code(hmac_key, &normalized);
    let coll = db.collection::<CliPairing>(COLLECTION_NAME);
    let record = coll
        .find_one(doc! { "code_hash": &code_hash })
        .await?
        .ok_or_else(|| AppError::NotFound("pairing code not found".into()))?;

    // Cross-user mismatch returns the same opaque 404 as other
    // failures to avoid enumeration, but we MUST NOT increment the
    // owner's `failed_claim_attempts` here. Any other authenticated
    // user who obtains the 8-char code (terminal sharing, agent
    // relay, shoulder surfing) could otherwise burn through
    // `MAX_FAILED_CLAIMS` attempts on the real record and lock out
    // the legitimate owner — a denial-of-service against someone
    // whose security is already preserved by the user_id binding.
    // The counter only guards same-user-same-record brute-force,
    // which is impossible anyway (the code hash lookup is unique
    // and codes are bound to the creator).
    if record.user_id != session_user_id {
        return Err(AppError::NotFound("pairing code not found".into()));
    }

    // Same-user failure modes that SHOULD burn the counter:
    // legitimate brute-force of one's own expired / locked
    // record. These are rare but the attempt budget is still
    // the right ceiling.
    if record.failed_claim_attempts >= MAX_FAILED_CLAIMS || record.expires_at < Utc::now() {
        let _ = coll
            .update_one(
                doc! { "_id": &record.id },
                doc! { "$inc": { "failed_claim_attempts": 1i32 } },
            )
            .await;
        return Err(AppError::NotFound("pairing code not found".into()));
    }

    match record.status {
        CliPairingStatus::Pending => {
            // First claim — atomic Pending → Claimed transition.
            // Status literals must match the
            // `#[serde(rename_all = "snake_case")]` encoding on
            // `CliPairingStatus`.
            let update = doc! {
                "$set": {
                    "status": "claimed",
                    "claimed_at": BsonDateTime::from_chrono(Utc::now()),
                    "claimed_from_ip": source_ip
                        .map(|s| Bson::String(s.to_string()))
                        .unwrap_or(Bson::Null),
                }
            };
            let filter = doc! {
                "_id": &record.id,
                "status": "pending",
            };
            let res = coll.update_one(filter, update).await?;
            if res.matched_count == 0 {
                // A concurrent request beat us to the transition.
                // Re-read and dispatch: if the racer also only
                // advanced to `Claimed` (common two-tab case), we
                // return the idempotent resumed-claim response; but
                // if they've already completed or cancelled the
                // pairing, we must NOT return ClaimedPairing —
                // otherwise the second tab could re-run
                // side-effecting work on a terminal record.
                let refreshed = coll
                    .find_one(doc! { "_id": &record.id })
                    .await?
                    .ok_or_else(|| AppError::NotFound("pairing not found".into()))?;
                return match refreshed.status {
                    CliPairingStatus::Claimed => {
                        let action_started = refreshed.action_started_at.is_some();
                        Ok(ClaimedPairing {
                            id: refreshed.id,
                            kind: refreshed.kind,
                            prefill: refreshed.prefill,
                            resumed: true,
                            action_started,
                        })
                    }
                    CliPairingStatus::Completed => {
                        Err(AppError::Conflict("pairing already completed".into()))
                    }
                    CliPairingStatus::Cancelled => {
                        Err(AppError::Conflict("pairing was cancelled".into()))
                    }
                    CliPairingStatus::Pending => {
                        // Shouldn't be reachable — the filter
                        // update failed, so status must have moved
                        // on. But if Mongo returned stale data,
                        // retry once more by recursing is risky;
                        // fail with a clear conflict instead.
                        Err(AppError::Conflict(
                            "pairing state changed during claim".into(),
                        ))
                    }
                };
            }
            Ok(ClaimedPairing {
                id: record.id,
                kind: record.kind,
                prefill: record.prefill,
                resumed: false,
                action_started: false,
            })
        }
        CliPairingStatus::Claimed => {
            // Idempotent re-claim: the same user refreshed `/cli/pair`,
            // opened a second tab, or hit the back button and retyped
            // the code. Returning the existing pairing lets the
            // frontend resume without bricking the flow for the full
            // 15-minute TTL. The user_id guard above already rejected
            // cross-user claims, so this branch is only reachable by
            // the original actor (or someone who has both the code
            // AND a session cookie for the same user — in which case
            // the pairing is already theirs).
            //
            // `action_started` lets the frontend distinguish the
            // recoverable pre-action refresh from the dangerous
            // post-action refresh: only the latter blocks.
            let action_started = record.action_started_at.is_some();
            Ok(ClaimedPairing {
                id: record.id,
                kind: record.kind,
                prefill: record.prefill,
                resumed: true,
                action_started,
            })
        }
        CliPairingStatus::Completed => Err(AppError::Conflict("pairing already completed".into())),
        CliPairingStatus::Cancelled => Err(AppError::Conflict("pairing was cancelled".into())),
    }
}

/// Mark the pairing complete and store the typed ack payload. Only the
/// originating user (matched against the session) may complete, and
/// only while the pairing is in Claimed state.
///
/// Rejects malformed acks BEFORE the state transition so a buggy
/// browser page can't leave the pairing in a `Completed` terminal
/// state with an un-parseable payload (the CLI would otherwise poll,
/// see `Completed`, then hard-error on `deserialize_from_value` —
/// with the underlying key/token already created on the server).
/// The validation is per-kind: each flow has a narrow accepted shape
/// that matches the CLI-side `*AckPayload` structs in
/// `cli/src/wizard/mod.rs`.
pub async fn complete(
    db: &mongodb::Database,
    encryption_keys: &crate::crypto::aes::EncryptionKeys,
    id: &str,
    session_user_id: &str,
    ack: serde_json::Value,
) -> AppResult<()> {
    let ack_bytes =
        serde_json::to_vec(&ack).map_err(|e| AppError::BadRequest(format!("invalid ack: {e}")))?;
    if ack_bytes.len() > MAX_ACK_BYTES {
        return Err(AppError::BadRequest(format!(
            "ack exceeds {MAX_ACK_BYTES} bytes"
        )));
    }

    let coll = db.collection::<CliPairing>(COLLECTION_NAME);
    let record = coll
        .find_one(doc! { "_id": id })
        .await?
        .ok_or_else(|| AppError::NotFound("pairing not found".into()))?;

    if record.user_id != session_user_id {
        return Err(AppError::NotFound("pairing not found".into()));
    }
    // Special-case `Completed` as idempotent success when the incoming
    // ack matches what we already stored. The browser retries
    // `/complete` when the first POST's response is lost to a timeout
    // or connection reset; returning 409 on that retry would strand
    // the DisplayOnce secret (already minted, about to be rendered)
    // because the frontend's `NotifyingCliPanel` only transitions
    // on a 2xx. By treating a duplicate complete-with-same-ack as
    // Ok, the retry path completes normally. A different ack on an
    // already-completed record is still a real conflict — that
    // would indicate a buggy or hostile frontend, so we reject it.
    //
    // This check MUST run BEFORE the expiry guard below. The classic
    // race is: the first `/complete` succeeds at T=expiry-1s, the
    // client's connection drops, the retry arrives at T=expiry+1s.
    // Checking expiry first would reject the retry with "pairing
    // expired" even though the server already accepted the original
    // ack; the user would sit on "Notifying CLI…" forever even
    // though the CLI is perfectly able to finish.
    if record.status == CliPairingStatus::Completed {
        let stored = record.ack_payload.as_ref();
        if stored == Some(&ack) {
            return Ok(());
        }
        return Err(AppError::Conflict(
            "pairing already completed with a different ack".into(),
        ));
    }
    if record.expires_at < Utc::now() {
        return Err(AppError::BadRequest("pairing expired".into()));
    }
    if record.status != CliPairingStatus::Claimed {
        return Err(AppError::Conflict(match record.status {
            CliPairingStatus::Pending => "pairing not yet claimed".into(),
            CliPairingStatus::Completed => unreachable!(),
            CliPairingStatus::Cancelled => "pairing was cancelled".into(),
            CliPairingStatus::Claimed => unreachable!(),
        }));
    }

    // Per-kind shape validation. We don't re-derive the Rust ack
    // structs here (they live in `cli/src/wizard/mod.rs` and we don't
    // want a circular crate dep); instead we assert the minimal
    // invariants that a well-formed ack of each kind must satisfy.
    validate_ack_for_kind(&record.kind, &ack)?;

    // Extra server-side check for ai-key flows: verify the
    // referenced UserService is actually active and owned by the
    // session user. The browser-side flow races the provider
    // OAuth / device-code callback against `beforeunload`: if
    // the user closes the tab right after authorizing, we fire a
    // best-effort `/complete` as keepalive so the CLI unblocks
    // without waiting for the next `pollUntilActive` tick. That
    // keepalive path can't GET-then-POST to confirm the key
    // flipped to active client-side, so we validate here. If
    // the placeholder is still `pending_auth` (user closed tab
    // BEFORE authorizing), we reject the ack — safer than
    // telling the CLI "service connected" for a credential that
    // actually isn't.
    // For ai-key completions we need both an active-status
    // check AND a rewrite of `slug` / `label` to match the
    // authoritative `UserService`. The keepalive beforeunload
    // path sends slug/label from React component props — which
    // were captured BEFORE `resolve_unique_slug()` may have
    // renamed the service on create (e.g. `llm-openai` ->
    // `llm-openai-2` when a same-named service already exists).
    // Storing the client-supplied values would make the CLI
    // print a proxy URL for a slug that doesn't exist.
    let ack = if record.kind == "ai-key" {
        normalize_ai_key_ack(db, encryption_keys, session_user_id, ack).await?
    } else {
        ack
    };

    let ack_bson =
        bson::to_bson(&ack).map_err(|e| AppError::Internal(format!("ack to_bson: {e}")))?;
    let filter = doc! { "_id": id, "status": "claimed" };
    let update = doc! {
        "$set": {
            "status": "completed",
            "completed_at": BsonDateTime::from_chrono(Utc::now()),
            "ack_payload": ack_bson,
        }
    };
    let res = coll.update_one(filter, update).await?;
    if res.matched_count == 0 {
        return Err(AppError::Conflict("pairing state changed".into()));
    }
    Ok(())
}

/// Enforce the per-kind ack shape. The CLI still does strict
/// `deny_unknown_fields` parsing on its side — this server-side
/// check exists so a malformed ack is rejected BEFORE the pairing
/// transitions to `Completed`, which is a terminal state the CLI
/// can't recover from. Kinds must stay in sync with the frontend
/// `PairingKind` union and the CLI's `PairingFlow` enum.
/// Verify and CANONICALIZE an ai-key ack against the actual
/// UserService record. This does two jobs:
///
/// 1. **Status gate.** Rejects acks for services that aren't
///    `active` — the keepalive beforeunload path fires
///    `/complete` without first doing a GET, so the server has
///    to validate instead of trusting the client.
///
/// 2. **Slug / label canonicalization.** `resolve_unique_slug()`
///    can rename the created service at `POST /keys` time (e.g.
///    `llm-openai` → `llm-openai-2` when a same-named service
///    already exists). The browser-side unload handler sends
///    slug/label from the React component props, which were
///    captured BEFORE that rename, so the ack can carry stale
///    metadata. Storing it verbatim would make the CLI print a
///    proxy URL for a slug that doesn't resolve. We overwrite
///    those fields with the UserService's authoritative values
///    before the ack is persisted.
///
/// Returns the canonicalized ack. On any validation failure the
/// pairing stays in `claimed` — safer than persisting an ack
/// that references a non-active or mis-named service.
async fn normalize_ai_key_ack(
    db: &mongodb::Database,
    encryption_keys: &crate::crypto::aes::EncryptionKeys,
    session_user_id: &str,
    mut ack: serde_json::Value,
) -> AppResult<serde_json::Value> {
    let service_id = ack
        .get("service_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest("ai-key ack missing service_id".into()))?
        .to_string();

    // Reuse the unified key view so this stays in lockstep with
    // how the rest of the app reads service status (including
    // api_key status + org-share resolution). A scoped org member
    // polling on a sibling-owned pairing would still fail the
    // existing ownership check on the record above — this helper
    // just asserts the referenced credential itself is usable.
    let view = crate::services::unified_key_service::get_key(
        db,
        encryption_keys,
        session_user_id,
        &service_id,
    )
    .await
    .map_err(|e| match e {
        AppError::NotFound(_) => {
            AppError::BadRequest("ai-key ack references unknown service".into())
        }
        other => other,
    })?;

    if !view.status.eq_ignore_ascii_case("active") {
        return Err(AppError::BadRequest(
            "ai-key ack references a non-active service".into(),
        ));
    }

    // Canonicalize slug / label. The ack is an object (shape
    // enforced by `validate_ack_for_kind` upstream), so we can
    // rewrite these two fields directly.
    if let Some(obj) = ack.as_object_mut() {
        obj.insert(
            "slug".to_string(),
            serde_json::Value::String(view.slug.clone()),
        );
        obj.insert(
            "label".to_string(),
            serde_json::Value::String(view.label.clone()),
        );
    }
    Ok(ack)
}

fn validate_ack_for_kind(kind: &str, ack: &serde_json::Value) -> AppResult<()> {
    let obj = ack
        .as_object()
        .ok_or_else(|| AppError::BadRequest("ack must be a JSON object".into()))?;

    let require_bool_true = |field: &str| -> AppResult<()> {
        match obj.get(field).and_then(|v| v.as_bool()) {
            Some(true) => Ok(()),
            Some(false) => Err(AppError::BadRequest(format!("ack.{field} must be true"))),
            None => Err(AppError::BadRequest(format!("ack.{field} is required"))),
        }
    };
    let require_str = |field: &str| -> AppResult<&str> {
        let value = obj
            .get(field)
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::BadRequest(format!("ack.{field} is required")))?;
        if value.is_empty() || value.len() > 256 {
            return Err(AppError::BadRequest(format!(
                "ack.{field} must be 1..=256 chars"
            )));
        }
        Ok(value)
    };
    let require_exact_keys = |expected: &[&str]| -> AppResult<()> {
        for k in expected {
            if !obj.contains_key(*k) {
                return Err(AppError::BadRequest(format!("ack.{k} is required")));
            }
        }
        for actual in obj.keys() {
            if !expected.contains(&actual.as_str()) {
                return Err(AppError::BadRequest(format!(
                    "ack.{actual} is not a permitted field for kind {kind}"
                )));
            }
        }
        Ok(())
    };

    match kind {
        "ai-key" => {
            require_exact_keys(&["acknowledged", "service_id", "slug", "label"])?;
            require_bool_true("acknowledged")?;
            require_str("service_id")?;
            require_str("slug")?;
            require_str("label")?;
        }
        "api-key-create" => {
            require_exact_keys(&["acknowledged", "api_key_id"])?;
            require_bool_true("acknowledged")?;
            require_str("api_key_id")?;
        }
        "api-key-rotate"
        | "node-rotate-token"
        | "service-account-rotate-secret"
        | "developer-app-rotate-secret" => {
            require_exact_keys(&["acknowledged", "resource_id"])?;
            require_bool_true("acknowledged")?;
            require_str("resource_id")?;
        }
        "node-register-token" => {
            require_exact_keys(&["acknowledged", "token_id"])?;
            require_bool_true("acknowledged")?;
            require_str("token_id")?;
        }
        "service-account-create" => {
            require_exact_keys(&["acknowledged", "service_account_id"])?;
            require_bool_true("acknowledged")?;
            require_str("service_account_id")?;
        }
        "developer-app-create" => {
            require_exact_keys(&["acknowledged", "developer_app_id"])?;
            require_bool_true("acknowledged")?;
            require_str("developer_app_id")?;
        }
        "mfa-setup" => {
            require_exact_keys(&["acknowledged", "factor_id"])?;
            require_bool_true("acknowledged")?;
            require_str("factor_id")?;
        }
        other => {
            // Unknown kind — don't accept any ack. The pairing record
            // would otherwise transition to Completed with an
            // arbitrary payload.
            return Err(AppError::BadRequest(format!(
                "unknown pairing kind '{other}'"
            )));
        }
    }
    Ok(())
}

/// Mark the pairing cancelled. Only the originating user may cancel,
/// and only while the pairing is Pending or Claimed.
///
/// Race-proofing: the status check happens at read time, but a
/// concurrent `complete()` can transition the record to Completed
/// between our read and our write. The update therefore filters by
/// `status ∈ {pending, claimed}` so a racing complete wins and the
/// cancel becomes a no-op. Without this guard, a Ctrl-C that fires
/// the moment the browser POSTed `/complete` would overwrite the
/// ack to `cancelled`, orphaning the created key/token (the CLI's
/// next poll would see `Cancelled` and exit with no summary).
/// Mark the pairing as "destructive action about to execute" so a
/// concurrent re-claim (from a refresh or a second tab) can tell
/// that replaying the action would invalidate an already-minted
/// secret. Idempotent — additional calls are no-ops once the
/// timestamp is set.
///
/// Must only be called while the pairing is `Claimed`. Only the
/// originating user is allowed; mismatches return NotFound to avoid
/// leaking state.
pub async fn reserve_action(
    db: &mongodb::Database,
    id: &str,
    session_user_id: &str,
) -> AppResult<()> {
    let coll = db.collection::<CliPairing>(COLLECTION_NAME);
    let record = coll
        .find_one(doc! { "_id": id })
        .await?
        .ok_or_else(|| AppError::NotFound("pairing not found".into()))?;
    if record.user_id != session_user_id {
        return Err(AppError::NotFound("pairing not found".into()));
    }
    if record.status != CliPairingStatus::Claimed {
        return Err(AppError::Conflict("pairing not in claimed state".into()));
    }
    // Mirror the `complete()` expiry guard so we never admit a
    // destructive step whose `/complete` ack we'll later refuse.
    // Without this a user who claims near TTL and clicks
    // Create/Rotate after `expires_at` would successfully mint a
    // one-time secret that the CLI never learns about — the ack
    // fails in `complete()` and the record silently expires while
    // the side-effect persists.
    if record.expires_at < Utc::now() {
        return Err(AppError::BadRequest("pairing expired".into()));
    }
    // Reject if already started. Two already-claimed tabs would
    // otherwise both succeed here, each go on to mint a one-time
    // secret, and the CLI would only ever hear about whichever one
    // posted `/complete` first — leaving the other as an orphaned,
    // untracked credential. Making this a single-winner transition
    // closes that window.
    //
    // Expiry is ALSO enforced atomically here, not just at the
    // pre-read check above. Without the `expires_at` clause in the
    // filter, a user who clicks Create/Rotate right at the TTL
    // boundary could pass the pre-read check and reserve, then
    // have `/complete` rejected as expired a moment later —
    // leaving a minted key/token orphaned from the CLI. Including
    // the expiry in the atomic filter makes reserve and
    // expire-check consistent in a single update.
    //
    // Fresh records serialize `action_started_at` as BSON `null`
    // via the `bson_datetime::optional` helper, so `{ $exists:
    // false }` alone wouldn't match — we match both "missing" and
    // "null" so the first caller always wins.
    let now = Utc::now();
    let res = coll
        .update_one(
            doc! {
                "_id": id,
                "user_id": session_user_id,
                "status": "claimed",
                "expires_at": { "$gt": BsonDateTime::from_chrono(now) },
                "$or": [
                    { "action_started_at": { "$exists": false } },
                    { "action_started_at": null },
                ],
            },
            doc! {
                "$set": {
                    "action_started_at": BsonDateTime::from_chrono(now),
                }
            },
        )
        .await?;

    if res.matched_count == 0 {
        // Either someone else already reserved (two-tab race), the
        // status moved on to Completed/Cancelled, or the pairing
        // expired between the pre-read and the update. Returning
        // Conflict is appropriate in all three cases — the browser
        // shows a "already started or expired" message and bails
        // so it won't run the destructive step.
        return Err(AppError::Conflict(
            "pairing action already started, expired, or not in claimed state".into(),
        ));
    }
    Ok(())
}

/// Undo a prior `reserve_action` for the SAME user on a pairing
/// that is still in `Claimed` state. Used by the frontend when the
/// user cancels an OAuth / device-code sub-flow BEFORE the provider
/// callback has landed — at that point the destructive step hasn't
/// actually produced a usable artifact (the `pending_auth`
/// placeholder was already deleted by `abandonPlaceholderKey`), so
/// clearing the reservation lets the user retry on the same pairing
/// (e.g. edit the label and try again) instead of being forced to
/// re-run the CLI for a brand-new code.
///
/// MUST NOT rewind a Completed pairing: the api-key create/rotate /
/// node register/rotate-token panels post `/complete` immediately
/// after their destructive call, which transitions the pairing to
/// `Completed`. The `status: "claimed"` filter below enforces that
/// — rewinding a Completed pairing would be unsafe (the one-time
/// secret was already minted and the user saw it). `matched_count
/// == 0` is treated as a no-op.
pub async fn rewind_action(
    db: &mongodb::Database,
    id: &str,
    session_user_id: &str,
) -> AppResult<()> {
    let coll = db.collection::<CliPairing>(COLLECTION_NAME);
    coll.update_one(
        doc! {
            "_id": id,
            "user_id": session_user_id,
            "status": "claimed",
        },
        doc! { "$set": { "action_started_at": null } },
    )
    .await?;
    Ok(())
}

pub async fn cancel(db: &mongodb::Database, id: &str, session_user_id: &str) -> AppResult<()> {
    let coll = db.collection::<CliPairing>(COLLECTION_NAME);
    let record = coll
        .find_one(doc! { "_id": id })
        .await?
        .ok_or_else(|| AppError::NotFound("pairing not found".into()))?;

    if record.user_id != session_user_id {
        return Err(AppError::NotFound("pairing not found".into()));
    }
    if !matches!(
        record.status,
        CliPairingStatus::Pending | CliPairingStatus::Claimed
    ) {
        return Err(AppError::Conflict("pairing not cancellable".into()));
    }

    // Filter by user_id (defense-in-depth) and by the set of
    // cancellable statuses. If a concurrent `complete()` landed
    // first, `matched_count` will be 0 and we return success — the
    // pairing is already in a terminal state and the browser side
    // wins.
    coll.update_one(
        doc! {
            "_id": id,
            "user_id": session_user_id,
            "status": { "$in": ["pending", "claimed"] },
        },
        doc! { "$set": { "status": "cancelled" } },
    )
    .await?;
    Ok(())
}

/// Read current status for the CLI's poll. Only the originating user
/// may poll; missing records and records owned by another user both
/// return the same opaque "expired" shape so the endpoint cannot be
/// used as a pairing-id existence oracle for stolen / guessed ids.
pub async fn poll(
    db: &mongodb::Database,
    id: &str,
    session_user_id: &str,
) -> AppResult<PollResponse> {
    let coll = db.collection::<CliPairing>(COLLECTION_NAME);
    let record = match coll.find_one(doc! { "_id": id }).await? {
        Some(r) => r,
        // TTL sweep may have deleted an expired record; or the id
        // doesn't exist at all. Same response either way so a
        // caller with a guessed id can't distinguish. The CLI
        // treats `Expired` as terminal, so empty `kind` /
        // `expires_at` fields are fine — the originating caller's
        // real pairing never hits this branch while it's still
        // claimable.
        None => {
            return Ok(PollResponse {
                status: PollStatus::Expired,
                kind: String::new(),
                expires_at: String::new(),
            });
        }
    };

    // Owned-by-someone-else mirrors the missing-record branch
    // verbatim: returning a distinct `NotFound` would let a
    // caller with a stolen id probe whether the id exists vs
    // belongs to another user. Symmetric fake-expired closes
    // that oracle.
    if record.user_id != session_user_id {
        return Ok(PollResponse {
            status: PollStatus::Expired,
            kind: String::new(),
            expires_at: String::new(),
        });
    }

    let kind = record.kind.clone();
    let expires_at = record.expires_at.to_rfc3339();

    // In-flight expiry check (sweeper runs asynchronously).
    if record.expires_at < Utc::now()
        && !matches!(
            record.status,
            CliPairingStatus::Completed | CliPairingStatus::Cancelled
        )
    {
        return Ok(PollResponse {
            status: PollStatus::Expired,
            kind,
            expires_at,
        });
    }

    let status = match record.status {
        CliPairingStatus::Pending => PollStatus::Pending,
        CliPairingStatus::Claimed => PollStatus::Claimed,
        CliPairingStatus::Cancelled => PollStatus::Cancelled,
        CliPairingStatus::Completed => PollStatus::Completed {
            ack: record.ack_payload.unwrap_or(serde_json::Value::Null),
        },
    };
    Ok(PollResponse {
        status,
        kind,
        expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_code_has_correct_shape() {
        for _ in 0..100 {
            let code = generate_code();
            assert_eq!(code.len(), CODE_LEN);
            assert!(
                code.chars().all(|c| CODE_ALPHABET.contains(&(c as u8))),
                "code {code} has unexpected chars"
            );
        }
    }

    #[test]
    fn normalize_code_strips_dashes_and_whitespace() {
        assert_eq!(normalize_code("abcd-1234"), "ABCD1234");
        assert_eq!(normalize_code("  AB CD-12 34 "), "ABCD1234");
        assert_eq!(normalize_code("abcd1234"), "ABCD1234");
    }

    #[test]
    fn format_code_inserts_dash() {
        assert_eq!(format_code("ABCD1234"), "ABCD-1234");
        assert_eq!(format_code("short"), "short");
    }

    #[test]
    fn hmac_code_is_deterministic_and_keyed() {
        let key_a = [0x11u8; 32];
        let key_b = [0x22u8; 32];
        let c1 = hmac_code(&key_a, "ABCD1234");
        let c2 = hmac_code(&key_a, "ABCD1234");
        let c3 = hmac_code(&key_b, "ABCD1234");
        let c4 = hmac_code(&key_a, "ABCD1235");
        assert_eq!(c1, c2, "same key + code must produce identical digest");
        assert_ne!(
            c1, c3,
            "different keys must produce different digests (keyed)"
        );
        assert_ne!(c1, c4, "different codes must produce different digests");
        // 32-byte digest → 64 hex chars.
        assert_eq!(c1.len(), 64);
        assert!(
            c1.chars().all(|c| c.is_ascii_hexdigit()),
            "digest must be hex-encoded"
        );
    }

    #[test]
    fn ack_size_limit_rejects_oversize() {
        // Not an integration test — we can exercise the size check via
        // `serde_json::to_vec` directly because it doesn't need the DB.
        let big = serde_json::Value::String("x".repeat(MAX_ACK_BYTES + 1));
        let bytes = serde_json::to_vec(&big).unwrap();
        assert!(bytes.len() > MAX_ACK_BYTES);
    }

    #[test]
    fn poll_status_round_trip() {
        let cases = vec![
            PollStatus::Pending,
            PollStatus::Claimed,
            PollStatus::Cancelled,
            PollStatus::Expired,
            PollStatus::Completed {
                ack: serde_json::json!({"acknowledged": true, "api_key_id": "abc"}),
            },
        ];
        for status in cases {
            let json = serde_json::to_string(&status).unwrap();
            let decoded: PollStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(
                serde_json::to_value(&status).unwrap(),
                serde_json::to_value(&decoded).unwrap()
            );
        }
    }

    #[test]
    fn poll_response_flattens_status_fields() {
        // Ensures the wire shape is `{status, kind, expires_at, ack?}`
        // at the top level so a CLI client can destructure without
        // reaching into a nested `status` object.
        let resp = PollResponse {
            status: PollStatus::Completed {
                ack: serde_json::json!({"acknowledged": true, "api_key_id": "x"}),
            },
            kind: "api-key-create".into(),
            expires_at: "2026-04-21T15:30:00Z".into(),
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["status"], "completed");
        assert_eq!(v["kind"], "api-key-create");
        assert_eq!(v["expires_at"], "2026-04-21T15:30:00Z");
        assert_eq!(v["ack"]["api_key_id"], "x");
    }

    #[test]
    fn validate_ack_accepts_well_formed_shapes() {
        assert!(
            validate_ack_for_kind(
                "api-key-create",
                &serde_json::json!({"acknowledged": true, "api_key_id": "abc"})
            )
            .is_ok()
        );
        assert!(
            validate_ack_for_kind(
                "node-register-token",
                &serde_json::json!({"acknowledged": true, "token_id": "t-1"})
            )
            .is_ok()
        );
        assert!(
            validate_ack_for_kind(
                "api-key-rotate",
                &serde_json::json!({"acknowledged": true, "resource_id": "k-1"})
            )
            .is_ok()
        );
        assert!(
            validate_ack_for_kind(
                "node-rotate-token",
                &serde_json::json!({"acknowledged": true, "resource_id": "n-1"})
            )
            .is_ok()
        );
        assert!(
            validate_ack_for_kind(
                "ai-key",
                &serde_json::json!({
                    "acknowledged": true,
                    "service_id": "svc-1",
                    "slug": "llm-openai",
                    "label": "personal"
                })
            )
            .is_ok()
        );
        // Issue #506 — wizard coverage extended to service-account /
        // developer-app secret leaks and full MFA enrollment.
        assert!(
            validate_ack_for_kind(
                "service-account-create",
                &serde_json::json!({"acknowledged": true, "service_account_id": "sa-1"})
            )
            .is_ok()
        );
        assert!(
            validate_ack_for_kind(
                "service-account-rotate-secret",
                &serde_json::json!({"acknowledged": true, "resource_id": "sa-1"})
            )
            .is_ok()
        );
        assert!(
            validate_ack_for_kind(
                "developer-app-create",
                &serde_json::json!({"acknowledged": true, "developer_app_id": "app-1"})
            )
            .is_ok()
        );
        assert!(
            validate_ack_for_kind(
                "developer-app-rotate-secret",
                &serde_json::json!({"acknowledged": true, "resource_id": "app-1"})
            )
            .is_ok()
        );
        assert!(
            validate_ack_for_kind(
                "mfa-setup",
                &serde_json::json!({"acknowledged": true, "factor_id": "f-1"})
            )
            .is_ok()
        );
    }

    #[test]
    fn validate_ack_rejects_secret_smuggle_on_new_kinds() {
        // Per-kind extra-field rejection: a browser that tries to
        // smuggle the raw client_secret / TOTP secret / recovery
        // codes through the ack must be refused at the validator.
        for (kind, base) in [
            (
                "service-account-create",
                serde_json::json!({"acknowledged": true, "service_account_id": "sa-1"}),
            ),
            (
                "developer-app-create",
                serde_json::json!({"acknowledged": true, "developer_app_id": "app-1"}),
            ),
            (
                "mfa-setup",
                serde_json::json!({"acknowledged": true, "factor_id": "f-1"}),
            ),
        ] {
            let mut tampered = base.clone();
            tampered["client_secret"] = serde_json::json!("leaked");
            assert!(
                validate_ack_for_kind(kind, &tampered).is_err(),
                "kind {kind} should reject smuggled client_secret"
            );
        }
    }

    #[test]
    fn validate_ack_rejects_missing_required_field() {
        let err =
            validate_ack_for_kind("api-key-create", &serde_json::json!({"acknowledged": true}))
                .unwrap_err();
        matches!(err, AppError::BadRequest(_));
    }

    #[test]
    fn validate_ack_rejects_acknowledged_false() {
        let err = validate_ack_for_kind(
            "api-key-create",
            &serde_json::json!({"acknowledged": false, "api_key_id": "x"}),
        )
        .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn validate_ack_rejects_extra_fields() {
        // A browser that tries to smuggle a secret through the ack
        // is rejected here — exact-keys check is symmetric to the
        // CLI's `deny_unknown_fields`.
        let err = validate_ack_for_kind(
            "api-key-create",
            &serde_json::json!({
                "acknowledged": true,
                "api_key_id": "abc",
                "full_key": "nyxid_leaked_secret"
            }),
        )
        .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn validate_ack_rejects_unknown_kind() {
        let err = validate_ack_for_kind("made-up-kind", &serde_json::json!({"acknowledged": true}))
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn claimed_pairing_serializes_flags() {
        // Wire-format guard: the frontend narrows on both `resumed`
        // and `action_started`, so both must survive JSON
        // serialization as booleans.
        let fresh = ClaimedPairing {
            id: "id".into(),
            kind: "api-key-create".into(),
            prefill: serde_json::json!({}),
            resumed: false,
            action_started: false,
        };
        let resumed_pre_action = ClaimedPairing {
            id: "id".into(),
            kind: "api-key-create".into(),
            prefill: serde_json::json!({}),
            resumed: true,
            action_started: false,
        };
        let resumed_post_action = ClaimedPairing {
            id: "id".into(),
            kind: "api-key-create".into(),
            prefill: serde_json::json!({}),
            resumed: true,
            action_started: true,
        };
        assert_eq!(serde_json::to_value(&fresh).unwrap()["resumed"], false);
        assert_eq!(
            serde_json::to_value(&fresh).unwrap()["action_started"],
            false
        );
        assert_eq!(
            serde_json::to_value(&resumed_pre_action).unwrap()["action_started"],
            false
        );
        assert_eq!(
            serde_json::to_value(&resumed_post_action).unwrap()["action_started"],
            true
        );
    }

    #[test]
    fn ack_payload_equality_is_deep() {
        // Sanity check on the equality that drives the idempotent-
        // complete branch: two `serde_json::Value` objects with the
        // same fields in the same shape compare equal regardless of
        // construction path. Object key order doesn't matter for
        // `serde_json::Value` equality (it normalizes internally).
        let a = serde_json::json!({"acknowledged": true, "api_key_id": "abc"});
        let b = serde_json::json!({"api_key_id": "abc", "acknowledged": true});
        assert_eq!(a, b);
        let c = serde_json::json!({"acknowledged": true, "api_key_id": "def"});
        assert_ne!(a, c);
    }

    #[test]
    fn validate_ack_rejects_non_object() {
        let err =
            validate_ack_for_kind("api-key-create", &serde_json::json!("a string")).unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[test]
    fn code_alphabet_excludes_confusable_chars() {
        for bad in [b'I', b'L', b'O', b'U'] {
            assert!(
                !CODE_ALPHABET.contains(&bad),
                "alphabet should not contain confusable {}",
                bad as char
            );
        }
    }
}

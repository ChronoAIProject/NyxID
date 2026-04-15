use std::collections::{HashMap, HashSet};

use chrono::Utc;
use mongodb::bson::{self, doc};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};
use rand::Rng;
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::invite_code::{COLLECTION_NAME, InviteCode, InviteCodeUsage};
use crate::models::user::COLLECTION_NAME as USERS_COLLECTION;

/// Minimal user projection used to enrich invite code usage entries in admin
/// responses. Built from the `users` collection via a single batch lookup.
#[derive(Clone, Debug)]
pub struct InviteCodeUsageUser {
    pub email: String,
    pub display_name: Option<String>,
}

/// Narrow shape used by [`fetch_usage_users`] when it queries the `users`
/// collection. Deserializing into this instead of the full `User` model means:
///   1. Only `_id`, `email`, and `display_name` come over the wire (combined
///      with the `.projection(...)` call below).
///   2. Sensitive fields like `password_hash` and MFA secrets never get pulled
///      into admin process memory just to render a usage table.
///   3. Schema drift on unrelated `User` fields cannot crash the admin invite
///      code page — the projection is decoupled from the canonical model.
#[derive(Debug, serde::Deserialize)]
struct UserUsageProjection {
    #[serde(rename = "_id")]
    id: String,
    email: String,
    #[serde(default)]
    display_name: Option<String>,
}

/// Result of [`list_invite_codes`] — the codes themselves plus a lookup map
/// from `user_id` to minimal user details for every user referenced by any
/// `usages` entry. Users that have been deleted since the usage was recorded
/// will be absent from the map; callers should treat the mapping as optional.
pub struct InviteCodesWithUsers {
    pub codes: Vec<InviteCode>,
    pub users: HashMap<String, InviteCodeUsageUser>,
}

/// Fixed prefix used for all generated codes so admins can visually
/// distinguish an invite code from other credentials in logs / UI.
pub const CODE_PREFIX: &str = "NYX-";

/// Length (in characters) of the random suffix appended after `CODE_PREFIX`.
const CODE_SUFFIX_LEN: usize = 8;

/// Character set used to generate the random suffix. Excludes lowercase and
/// ambiguous characters to keep codes readable when shared verbally.
const CODE_CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// Number of attempts when retrying `insert_one` on a unique-index collision.
/// 36^8 ≈ 2.8 trillion combinations so in practice the first attempt always
/// succeeds; the retry loop guards against the pathological case of repeated
/// collisions rather than leaking a raw DB error to the admin.
const CODE_INSERT_MAX_ATTEMPTS: usize = 5;

/// Generate a code like "NYX-XXXXXXXX" (8 random alphanumeric uppercase chars).
pub fn generate_code() -> String {
    let mut rng = rand::thread_rng();
    let suffix: String = (0..CODE_SUFFIX_LEN)
        .map(|_| {
            let idx = rng.gen_range(0..CODE_CHARSET.len());
            CODE_CHARSET[idx] as char
        })
        .collect();
    format!("{CODE_PREFIX}{suffix}")
}

/// Normalize an invite code to its canonical storage form.
///
/// Codes are stored uppercased and trimmed so that user input is matched
/// regardless of whitespace or casing. This pairs with the frontend's
/// uppercase-on-type behavior and the `nyxid` CLI's normalization.
pub fn normalize_code(input: &str) -> String {
    input.trim().to_uppercase()
}

/// Return true if the given MongoDB error represents a unique-index violation.
fn is_duplicate_key_error(e: &mongodb::error::Error) -> bool {
    matches!(
        e.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(we))
            if we.code == 11000
    )
}

/// Create a new invite code.
///
/// Retries on unique-index collision (extremely unlikely in practice but
/// possible in principle, and we never want to surface a raw DB error to the
/// admin for this branch).
pub async fn create_invite_code(
    db: &mongodb::Database,
    created_by: &str,
    max_uses: i32,
    note: Option<&str>,
) -> AppResult<InviteCode> {
    let collection = db.collection::<InviteCode>(COLLECTION_NAME);

    for attempt in 0..CODE_INSERT_MAX_ATTEMPTS {
        let now = Utc::now();
        let invite = InviteCode {
            id: Uuid::new_v4().to_string(),
            code: generate_code(),
            max_uses,
            used_count: 0,
            created_by: created_by.to_string(),
            note: note.map(String::from),
            is_active: true,
            created_at: now,
            updated_at: now,
            usages: Vec::new(),
        };

        match collection.insert_one(&invite).await {
            Ok(_) => return Ok(invite),
            Err(e) if is_duplicate_key_error(&e) => {
                tracing::warn!(
                    attempt = attempt + 1,
                    code = %invite.code,
                    "Invite code generation collision, retrying"
                );
                continue;
            }
            Err(e) => return Err(AppError::DatabaseError(e)),
        }
    }

    Err(AppError::Internal(
        "Failed to generate unique invite code after retries".to_string(),
    ))
}

/// Atomically reserve one slot on an invite code.
///
/// Returns `Ok(invite_code_id)` on success, or `Err(AppError::BadRequest)` if
/// the code is unknown, inactive, or exhausted. This only touches `used_count`;
/// the caller is responsible for recording the usage (via `record_usage`) once
/// the downstream user creation succeeds, and releasing (via `release_reservation`)
/// if it fails.
pub async fn reserve_invite_code(db: &mongodb::Database, code: &str) -> AppResult<String> {
    let normalized = normalize_code(code);
    let now = bson::DateTime::from_chrono(Utc::now());

    let options = FindOneAndUpdateOptions::builder()
        .return_document(ReturnDocument::After)
        .build();

    let result = db
        .collection::<InviteCode>(COLLECTION_NAME)
        .find_one_and_update(
            doc! {
                "code": &normalized,
                "is_active": true,
                "$expr": { "$lt": ["$used_count", "$max_uses"] },
            },
            doc! {
                "$inc": { "used_count": 1 },
                "$set": { "updated_at": now },
            },
        )
        .with_options(options)
        .await?;

    match result {
        Some(invite) => Ok(invite.id),
        None => Err(AppError::BadRequest(
            "Invalid or exhausted invite code".to_string(),
        )),
    }
}

/// Record which user redeemed a previously-reserved invite code.
///
/// Best-effort append to the `usages` array. Failures are logged but do not
/// propagate: the user has already been successfully created, and the atomic
/// reservation still accurately reflects that one slot was consumed.
pub async fn record_usage(db: &mongodb::Database, invite_code_id: &str, user_id: &str) {
    let usage = InviteCodeUsage {
        user_id: user_id.to_string(),
        used_at: Utc::now(),
    };
    let usage_bson = match bson::to_bson(&usage) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(
                error = %e,
                invite_code_id = %invite_code_id,
                "Failed to serialize invite code usage"
            );
            return;
        }
    };

    let now = bson::DateTime::from_chrono(Utc::now());

    if let Err(e) = db
        .collection::<InviteCode>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": invite_code_id },
            doc! {
                "$push": { "usages": usage_bson },
                "$set": { "updated_at": now },
            },
        )
        .await
    {
        tracing::error!(
            error = %e,
            invite_code_id = %invite_code_id,
            user_id = %user_id,
            "Failed to record invite code usage"
        );
    }
}

/// Release a previously-reserved slot, decrementing `used_count`.
///
/// Used to compensate when `reserve_invite_code` has succeeded but the
/// downstream user creation fails (or silently no-ops for email enumeration
/// protection). Best-effort: errors are logged but not surfaced to the caller
/// because the user-facing request has already failed for an unrelated reason.
pub async fn release_reservation(db: &mongodb::Database, invite_code_id: &str) {
    let now = bson::DateTime::from_chrono(Utc::now());

    if let Err(e) = db
        .collection::<InviteCode>(COLLECTION_NAME)
        .update_one(
            doc! {
                "_id": invite_code_id,
                "used_count": { "$gt": 0 },
            },
            doc! {
                "$inc": { "used_count": -1 },
                "$set": { "updated_at": now },
            },
        )
        .await
    {
        tracing::error!(
            error = %e,
            invite_code_id = %invite_code_id,
            "Failed to release invite code reservation"
        );
    }
}

/// Batch-fetch minimal user details for every user referenced by any code in
/// the input slice. Collects both the `usages[n].user_id` redemption set AND
/// `created_by` admin ids into a single `$in` query over the `users`
/// collection — one round-trip, de-duplicated via HashSet (cheap when an admin
/// both creates and redeems the same code). Users deleted since they appeared
/// in an invite code simply won't appear in the returned map — callers must
/// treat the mapping as best-effort for both redemption and creator resolution.
///
/// Uses an explicit projection + a tight [`UserUsageProjection`] struct so the
/// admin invite-code path never pulls password hashes or other sensitive User
/// fields into memory and is insulated from unrelated `User` schema drift.
pub async fn fetch_usage_users(
    db: &mongodb::Database,
    codes: &[InviteCode],
) -> AppResult<HashMap<String, InviteCodeUsageUser>> {
    use futures::TryStreamExt;

    let user_ids: HashSet<String> = codes
        .iter()
        .flat_map(|ic| {
            ic.usages
                .iter()
                .map(|u| u.user_id.clone())
                .chain(std::iter::once(ic.created_by.clone()))
        })
        .collect();

    if user_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let user_ids_vec: Vec<String> = user_ids.into_iter().collect();
    let cursor = db
        .collection::<UserUsageProjection>(USERS_COLLECTION)
        .find(doc! { "_id": { "$in": &user_ids_vec } })
        .projection(doc! { "_id": 1, "email": 1, "display_name": 1 })
        .await?;

    let fetched: Vec<UserUsageProjection> = cursor.try_collect().await?;
    Ok(fetched
        .into_iter()
        .map(|u| {
            (
                u.id,
                InviteCodeUsageUser {
                    email: u.email,
                    display_name: u.display_name,
                },
            )
        })
        .collect())
}

/// List all invite codes (admin) together with a lookup map of the users
/// referenced by any `usages` entry. See [`fetch_usage_users`] for the user
/// resolution strategy.
pub async fn list_invite_codes(db: &mongodb::Database) -> AppResult<InviteCodesWithUsers> {
    use futures::TryStreamExt;

    let cursor = db
        .collection::<InviteCode>(COLLECTION_NAME)
        .find(doc! {})
        .sort(doc! { "created_at": -1 })
        .await?;

    let codes: Vec<InviteCode> = cursor.try_collect().await?;
    let users = fetch_usage_users(db, &codes).await?;

    Ok(InviteCodesWithUsers { codes, users })
}

/// Update the freeform `note` on an invite code.
///
/// The `note` argument is authoritative — whatever the caller passes becomes
/// the new value. `Some("text")` sets the note to that text; `Some("")` and
/// `None` both clear it (stored as `null` on the document). Returns the
/// freshly-updated `InviteCode` so the handler can render it back to the
/// admin without an extra read.
pub async fn update_invite_code_note(
    db: &mongodb::Database,
    id: &str,
    note: Option<String>,
) -> AppResult<InviteCode> {
    let now = bson::DateTime::from_chrono(Utc::now());

    let note_bson = match note.as_deref() {
        Some("") | None => bson::Bson::Null,
        Some(text) => bson::Bson::String(text.to_string()),
    };

    let options = FindOneAndUpdateOptions::builder()
        .return_document(ReturnDocument::After)
        .build();

    let updated = db
        .collection::<InviteCode>(COLLECTION_NAME)
        .find_one_and_update(
            doc! { "_id": id },
            doc! {
                "$set": {
                    "note": note_bson,
                    "updated_at": now,
                },
            },
        )
        .with_options(options)
        .await?;

    updated.ok_or_else(|| AppError::NotFound("Invite code not found".to_string()))
}

/// Deactivate an invite code by ID.
pub async fn deactivate_invite_code(db: &mongodb::Database, id: &str) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());

    let result = db
        .collection::<InviteCode>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": id },
            doc! { "$set": { "is_active": false, "updated_at": now } },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("Invite code not found".to_string()));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn generate_code_has_expected_format() {
        for _ in 0..100 {
            let code = generate_code();
            assert!(code.starts_with(CODE_PREFIX));
            let suffix = &code[CODE_PREFIX.len()..];
            assert_eq!(suffix.len(), CODE_SUFFIX_LEN);
            assert!(
                suffix.chars().all(|c| CODE_CHARSET.contains(&(c as u8))),
                "suffix contained invalid character: {suffix}"
            );
        }
    }

    #[test]
    fn generate_code_is_sufficiently_random() {
        // With 36^8 combinations, 1000 samples should all be unique in practice.
        let mut seen = HashSet::new();
        for _ in 0..1000 {
            let code = generate_code();
            assert!(
                seen.insert(code.clone()),
                "duplicate code generated within 1000 samples: {code}"
            );
        }
    }

    #[test]
    fn normalize_code_uppercases_and_trims() {
        assert_eq!(normalize_code("nyx-abc123"), "NYX-ABC123");
        assert_eq!(normalize_code("  NYX-abc123  "), "NYX-ABC123");
        assert_eq!(normalize_code("Nyx-AbC123"), "NYX-ABC123");
    }

    #[test]
    fn normalize_code_handles_empty_input() {
        assert_eq!(normalize_code(""), "");
        assert_eq!(normalize_code("   "), "");
    }

    #[test]
    fn normalize_matches_generated_code() {
        let generated = generate_code();
        // A newly-generated code is already in canonical form; normalizing
        // should be a no-op.
        assert_eq!(normalize_code(&generated), generated);
        // Lowercase variant of the same code should normalize to the generated form.
        assert_eq!(normalize_code(&generated.to_lowercase()), generated);
    }
}

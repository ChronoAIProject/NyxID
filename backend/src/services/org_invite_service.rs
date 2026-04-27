//! One-time org invite token issue and redemption.
//!
//! Distinct from `invite_code_service` (which gates new-user signup).
//! Org invites are scoped to a specific org, single-use, TTL-bound, and
//! redeemed by an existing logged-in person user.

use chrono::{Duration, Utc};
use mongodb::bson::{self, doc};
use rand::Rng;
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::org_invite::{COLLECTION_NAME, OrgInvite};
use crate::models::org_membership::{MemberScopeSource, OrgRole};
use crate::services::org_service;

/// Default TTL for new invites (24 hours). Configurable per-call via
/// [`create_invite`]'s `ttl` parameter.
pub const DEFAULT_INVITE_TTL: Duration = Duration::hours(24);

const NONCE_PREFIX: &str = "ORGINV-";
const NONCE_SUFFIX_LEN: usize = 24;
const NONCE_CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const NONCE_INSERT_MAX_ATTEMPTS: usize = 5;

/// Generate a URL-safe one-time token.
fn generate_nonce() -> String {
    let mut rng = rand::thread_rng();
    let suffix: String = (0..NONCE_SUFFIX_LEN)
        .map(|_| {
            let idx = rng.gen_range(0..NONCE_CHARSET.len());
            NONCE_CHARSET[idx] as char
        })
        .collect();
    format!("{NONCE_PREFIX}{suffix}")
}

fn is_duplicate_key_error(e: &mongodb::error::Error) -> bool {
    matches!(
        e.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(we))
            if we.code == 11000
    )
}

/// Issue a new invite for an org. The caller must be an admin of the org
/// (enforced at the handler level via [`org_service::is_admin`]).
pub async fn create_invite(
    db: &mongodb::Database,
    org_user_id: &str,
    created_by: &str,
    role: OrgRole,
    scope_source: MemberScopeSource,
    allowed_service_ids: Option<Vec<String>>,
    ttl: Option<Duration>,
) -> AppResult<OrgInvite> {
    // Verify the org exists and is an org.
    let _ = org_service::get_org_user(db, org_user_id).await?;

    let collection = db.collection::<OrgInvite>(COLLECTION_NAME);
    let now = Utc::now();
    let expires_at = now + ttl.unwrap_or(DEFAULT_INVITE_TTL);

    for attempt in 0..NONCE_INSERT_MAX_ATTEMPTS {
        let invite = OrgInvite {
            id: Uuid::new_v4().to_string(),
            org_user_id: org_user_id.to_string(),
            nonce: generate_nonce(),
            role,
            scope_source,
            allowed_service_ids: allowed_service_ids.clone(),
            created_by: created_by.to_string(),
            expires_at,
            redeemed_by: None,
            redeemed_at: None,
            created_at: now,
        };

        match collection.insert_one(&invite).await {
            Ok(_) => return Ok(invite),
            Err(e) if is_duplicate_key_error(&e) => {
                tracing::warn!(
                    attempt = attempt + 1,
                    "Org invite nonce collision, retrying"
                );
                continue;
            }
            Err(e) => return Err(AppError::DatabaseError(e)),
        }
    }

    Err(AppError::Internal(
        "Failed to generate unique invite nonce after retries".to_string(),
    ))
}

/// List invites issued for an org. Includes redeemed invites that have not
/// yet been removed by the TTL index. Caller must be admin (enforced upstream).
pub async fn list_invites_for_org(
    db: &mongodb::Database,
    org_user_id: &str,
) -> AppResult<Vec<OrgInvite>> {
    use futures::TryStreamExt;
    let cursor = db
        .collection::<OrgInvite>(COLLECTION_NAME)
        .find(doc! { "org_user_id": org_user_id })
        .await?;
    let invites: Vec<OrgInvite> = cursor.try_collect().await?;
    Ok(invites)
}

/// Cancel a pending invite by id. Removes the row entirely. Returns
/// `OrgInviteInvalid` if the invite does not exist or has already been
/// redeemed (deletion of a redeemed invite would lose audit information).
pub async fn cancel_invite(
    db: &mongodb::Database,
    org_user_id: &str,
    invite_id: &str,
) -> AppResult<()> {
    let collection = db.collection::<OrgInvite>(COLLECTION_NAME);
    let row = collection
        .find_one(doc! { "_id": invite_id, "org_user_id": org_user_id })
        .await?
        .ok_or_else(|| AppError::OrgInviteInvalid("invite not found".to_string()))?;

    if row.is_redeemed() {
        return Err(AppError::OrgInviteInvalid(
            "cannot cancel a redeemed invite".to_string(),
        ));
    }

    collection.delete_one(doc! { "_id": invite_id }).await?;
    Ok(())
}

/// Atomically redeem an invite for the given member. On success returns the
/// new [`crate::models::org_membership::OrgMembership`] and marks the invite
/// as redeemed.
pub async fn redeem_invite(
    db: &mongodb::Database,
    nonce: &str,
    member_user_id: &str,
) -> AppResult<crate::models::org_membership::OrgMembership> {
    let collection = db.collection::<OrgInvite>(COLLECTION_NAME);
    let now = Utc::now();
    let now_bson = bson::DateTime::from_chrono(now);

    // Atomically claim the invite (only if not yet redeemed and not expired).
    let claimed = collection
        .find_one_and_update(
            doc! {
                "nonce": nonce,
                "redeemed_at": bson::Bson::Null,
                "expires_at": { "$gt": now_bson },
            },
            doc! {
                "$set": {
                    "redeemed_by": member_user_id,
                    "redeemed_at": now_bson,
                }
            },
        )
        .await?;

    let invite = match claimed {
        Some(i) => i,
        None => {
            // Distinguish "no such invite" from "expired" / "already redeemed"
            // for better UX. One extra round-trip but worth it for clarity.
            let lookup = collection.find_one(doc! { "nonce": nonce }).await?;
            return Err(match lookup {
                None => AppError::OrgInviteInvalid("unknown invite".to_string()),
                Some(i) if i.is_redeemed() => {
                    AppError::OrgInviteInvalid("invite already redeemed".to_string())
                }
                Some(i) if i.is_expired(now) => AppError::OrgInviteExpired,
                Some(_) => AppError::OrgInviteInvalid("invite not redeemable".to_string()),
            });
        }
    };

    // Now create the membership. If membership creation fails (e.g. duplicate)
    // we rollback the redemption flag so the user can retry. This is a
    // best-effort rollback -- a hard crash between the two writes would leave
    // the invite marked redeemed without a membership, which is an
    // acceptable failure mode (admin can issue a fresh invite).
    let membership = match org_service::create_membership(
        db,
        &invite.org_user_id,
        member_user_id,
        invite.role,
        invite.scope_source,
        invite.allowed_service_ids.clone(),
    )
    .await
    {
        Ok(m) => m,
        Err(e) => {
            let _ = collection
                .update_one(
                    doc! { "_id": &invite.id },
                    doc! {
                        "$set": {
                            "redeemed_by": bson::Bson::Null,
                            "redeemed_at": bson::Bson::Null,
                        }
                    },
                )
                .await;
            return Err(e);
        }
    };

    Ok(membership)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonce_format() {
        let n = generate_nonce();
        assert!(n.starts_with(NONCE_PREFIX));
        assert_eq!(n.len(), NONCE_PREFIX.len() + NONCE_SUFFIX_LEN);
        assert!(
            n.chars()
                .skip(NONCE_PREFIX.len())
                .all(|c| { c.is_ascii_uppercase() || c.is_ascii_digit() })
        );
    }

    #[test]
    fn nonces_are_unique() {
        let a = generate_nonce();
        let b = generate_nonce();
        assert_ne!(a, b);
    }
}

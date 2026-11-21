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
use crate::services::{audit_service::AuditActor, org_membership_authority, org_service};
use mongodb::ClientSession;

/// Default TTL for new invites (24 hours). Configurable per-call via
/// [`create_invite_as`]'s `ttl` parameter.
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
#[cfg(test)]
pub async fn create_invite(
    db: &mongodb::Database,
    org_user_id: &str,
    created_by: &str,
    role: OrgRole,
    scope_source: MemberScopeSource,
    allowed_service_ids: Option<Vec<String>>,
    ttl: Option<Duration>,
) -> AppResult<OrgInvite> {
    let mut actor = org_service::audit::fixture_actor();
    actor.user_id = created_by.to_owned();
    create_invite_as(
        db,
        org_user_id,
        role,
        scope_source,
        allowed_service_ids,
        ttl,
        &actor,
    )
    .await
}

/// Issue an invite after verifying the issuer’s admin role in the membership transaction.
pub async fn create_invite_as(
    db: &mongodb::Database,
    org_user_id: &str,
    role: OrgRole,
    scope_source: MemberScopeSource,
    allowed_service_ids: Option<Vec<String>>,
    ttl: Option<Duration>,
    actor: &AuditActor,
) -> AppResult<OrgInvite> {
    // Verify the org exists and is an org.
    let _ = org_service::get_org_user(db, org_user_id).await?;

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
            created_by: actor.user_id.clone(),
            expires_at,
            redeemed_by: None,
            redeemed_at: None,
            created_at: now,
        };

        match insert_invite(db, &invite, actor).await {
            Ok(_) => return Ok(invite),
            Err(AppError::DatabaseError(e)) if is_duplicate_key_error(&e) => {
                tracing::warn!(
                    attempt = attempt + 1,
                    "Org invite nonce collision, retrying"
                );
                continue;
            }
            Err(e) => return Err(e),
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
#[cfg(test)]
pub async fn cancel_invite(
    db: &mongodb::Database,
    org_user_id: &str,
    invite_id: &str,
) -> AppResult<()> {
    cancel_invite_as(
        db,
        org_user_id,
        invite_id,
        &org_service::audit::fixture_actor(),
    )
    .await
}

/// Cancel a pending invite by id. Removes the row entirely. Returns
/// `OrgInviteInvalid` if the invite does not exist or has already been
/// redeemed (deletion of a redeemed invite would lose audit information).
pub async fn cancel_invite_as(
    db: &mongodb::Database,
    org_user_id: &str,
    invite_id: &str,
    actor: &AuditActor,
) -> AppResult<()> {
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result = cancel_in_session(db, org_user_id, invite_id, actor, &mut tx).await;
    org_membership_authority::finish(tx, result).await
}
async fn cancel_in_session(
    db: &mongodb::Database,
    org_user_id: &str,
    invite_id: &str,
    actor: &AuditActor,
    tx: &mut ClientSession,
) -> AppResult<()> {
    let collection = db.collection::<OrgInvite>(COLLECTION_NAME);
    let row = collection
        .find_one(doc! { "_id": invite_id, "org_user_id": org_user_id })
        .session(&mut *tx)
        .await?
        .ok_or_else(|| AppError::OrgInviteInvalid("invite not found".to_string()))?;

    if row.is_redeemed() {
        return Err(AppError::OrgInviteInvalid(
            "cannot cancel a redeemed invite".to_string(),
        ));
    }

    let removed = collection
        .delete_one(doc! { "_id": invite_id, "org_user_id":org_user_id, "redeemed_at":null })
        .session(&mut *tx)
        .await?;
    if removed.deleted_count != 1 {
        return Err(AppError::OrgInviteInvalid("invite not found".into()));
    }
    org_service::audit::invite(
        db,
        Some(actor),
        "org_invite_cancelled",
        org_user_id,
        invite_id,
        tx,
    )
    .await?;
    Ok(())
}
/// The membership transaction owns the organization fence. Returning only the
/// row ID keeps capability material out of the cancellation and audit path.
pub(crate) async fn retire_issuer_in_session(
    db: &mongodb::Database,
    org: &str,
    issuer: &str,
    actor: Option<&AuditActor>,
    tx: &mut ClientSession,
) -> AppResult<()> {
    let rows = db.collection::<bson::Document>(COLLECTION_NAME);
    let filter = doc! {
        "org_user_id": org, "redeemed_at": null, "expires_at": {"$gt": bson::DateTime::now()},
        "created_by": {"$in": [issuer, "", bson::Bson::Null]}
    };
    while let Some(row) = rows
        .find_one_and_delete(filter.clone())
        .projection(doc! {"_id":1})
        .session(&mut *tx)
        .await?
    {
        let id = row
            .get_str("_id")
            .map_err(|_| AppError::Internal("Invalid org invite ID".into()))?;
        org_service::audit::invite(db, actor, "org_invite_cancelled", org, id, tx).await?;
    }
    Ok(())
}

async fn fence_issuer(
    db: &mongodb::Database,
    invite: &OrgInvite,
    tx: &mut ClientSession,
) -> AppResult<()> {
    let invalid = || {
        AppError::OrgInviteInvalid("invite issuer is no longer an active organization admin".into())
    };
    if invite.created_by.is_empty() {
        return Err(invalid());
    }
    match org_membership_authority::fence_live_member(db, &invite.created_by, &mut *tx).await {
        Err(AppError::Conflict(_)) => return Err(invalid()),
        result => result?,
    }
    if db
        .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
        .find_one(
            doc! {"org_user_id": &invite.org_user_id, "member_user_id": &invite.created_by,
            "role": "admin", "revoked_at": null},
        )
        .projection(doc! {"_id":1})
        .session(tx)
        .await?
        .is_none()
    {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
pub async fn redeem_invite(
    db: &mongodb::Database,
    nonce: &str,
    member_user_id: &str,
) -> AppResult<crate::models::org_membership::OrgMembership> {
    redeem_invite_as(
        db,
        nonce,
        member_user_id,
        &org_service::audit::fixture_actor(),
    )
    .await
}

/// Atomically redeem an invite for the given member. On success returns the
/// new [`crate::models::org_membership::OrgMembership`] and marks the invite
/// as redeemed.
pub async fn redeem_invite_as(
    db: &mongodb::Database,
    nonce: &str,
    member_user_id: &str,
    actor: &AuditActor,
) -> AppResult<crate::models::org_membership::OrgMembership> {
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result = redeem_in_session(db, nonce, member_user_id, actor, &mut tx).await;
    let membership = org_membership_authority::finish(tx, result).await?;
    // The membership has committed; scheduled credits are best effort.
    crate::services::billing::schedules::eligibility::after_org_joined(
        db,
        &membership.org_user_id,
        &membership.member_user_id,
        membership.created_at,
    )
    .await;
    Ok(membership)
}
async fn redeem_in_session(
    db: &mongodb::Database,
    nonce: &str,
    member_user_id: &str,
    actor: &AuditActor,
    tx: &mut ClientSession,
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
        .session(&mut *tx)
        .await?;

    let invite = match claimed {
        Some(i) => i,
        None => {
            // Distinguish "no such invite" from "expired" / "already redeemed"
            // for better UX. One extra round-trip but worth it for clarity.
            let lookup = collection
                .find_one(doc! { "nonce": nonce })
                .session(&mut *tx)
                .await?;
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

    fence_issuer(db, &invite, tx).await?;

    // Reuse the ordinary membership preparation and fences. Its existing
    // transaction now also owns the invite claim and the required occurrence.
    let creation = org_service::membership::prepare_creation(
        db,
        &invite.org_user_id,
        member_user_id,
        invite.role,
        invite.scope_source,
        invite.allowed_service_ids,
        None,
    )
    .await?;
    let membership = org_service::membership::create_in_session(db, creation, tx).await?;
    org_service::audit::member(db, Some(actor), "org_member_joined", &membership, tx).await?;
    Ok(membership)
}

async fn insert_invite(
    db: &mongodb::Database,
    invite: &OrgInvite,
    actor: &AuditActor,
) -> AppResult<()> {
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result = insert_invite_in_session(db, invite, actor, &mut tx).await;
    org_membership_authority::finish(tx, result).await
}
async fn insert_invite_in_session(
    db: &mongodb::Database,
    invite: &OrgInvite,
    actor: &AuditActor,
    tx: &mut ClientSession,
) -> AppResult<()> {
    org_membership_authority::fence_org(db, &invite.org_user_id, &mut *tx).await?;
    fence_issuer(db, invite, tx).await?;
    db.collection::<OrgInvite>(COLLECTION_NAME)
        .insert_one(invite)
        .session(&mut *tx)
        .await?;
    org_service::audit::invite(
        db,
        Some(actor),
        "org_invite_created",
        &invite.org_user_id,
        &invite.id,
        tx,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::org_membership::COLLECTION_NAME as ORG_MEMBERSHIPS;
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::test_utils::{connect_test_database, test_user};
    use mongodb::IndexModel;
    use mongodb::bson::doc;
    use mongodb::options::IndexOptions;

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

    async fn insert_user(db: &mongodb::Database, id: &str, user_type: UserType) {
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(id, user_type))
            .await
            .expect("insert test user");
    }

    async fn insert_admin(db: &mongodb::Database, org: &str, admin: &str) {
        db.collection::<crate::models::org_membership::OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(crate::test_utils::test_membership(
                org,
                admin,
                OrgRole::Admin,
                None,
            ))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn create_list_and_cancel_invite_for_org() {
        let db = connect_test_database("orginv_crud")
            .await
            .expect("local MongoDB required for org_invite_service tests");
        let org_id = Uuid::new_v4().to_string();
        let other_org_id = Uuid::new_v4().to_string();
        let admin_id = Uuid::new_v4().to_string();
        insert_user(&db, &org_id, UserType::Org).await;
        insert_user(&db, &other_org_id, UserType::Org).await;
        insert_user(&db, &admin_id, UserType::Person).await;
        insert_admin(&db, &org_id, &admin_id).await;
        insert_admin(&db, &other_org_id, &admin_id).await;

        let invite = create_invite(
            &db,
            &org_id,
            &admin_id,
            OrgRole::Member,
            MemberScopeSource::Override,
            Some(vec!["svc-1".to_string(), "svc-2".to_string()]),
            Some(Duration::minutes(30)),
        )
        .await
        .expect("create invite");
        assert_eq!(invite.org_user_id, org_id);
        assert_eq!(invite.role, OrgRole::Member);
        assert_eq!(invite.scope_source, MemberScopeSource::Override);
        assert_eq!(
            invite.allowed_service_ids,
            Some(vec!["svc-1".to_string(), "svc-2".to_string()])
        );
        assert_eq!(invite.created_by, admin_id);
        assert!(!invite.is_redeemed());
        assert!(invite.expires_at > Utc::now());

        let other_invite = create_invite(
            &db,
            &other_org_id,
            &admin_id,
            OrgRole::Viewer,
            MemberScopeSource::Inherit,
            Some(vec!["ignored-for-inherit".to_string()]),
            None,
        )
        .await
        .expect("create other org invite");

        let org_invites = list_invites_for_org(&db, &org_id)
            .await
            .expect("list invites for org");
        assert_eq!(org_invites.len(), 1);
        assert_eq!(org_invites[0].id, invite.id);

        cancel_invite(&db, &org_id, &invite.id)
            .await
            .expect("cancel pending invite");
        let remaining = list_invites_for_org(&db, &org_id)
            .await
            .expect("list after cancel");
        assert!(remaining.is_empty());

        let other_remaining = list_invites_for_org(&db, &other_org_id)
            .await
            .expect("other org invite remains");
        assert_eq!(other_remaining.len(), 1);
        assert_eq!(other_remaining[0].id, other_invite.id);
    }

    #[tokio::test]
    async fn invite_claim_requires_attributed_live_admin_and_preserves_pending_row() {
        let db = connect_test_database("orginv_live_issuer").await.unwrap();
        let org = Uuid::new_v4().to_string();
        let admin = Uuid::new_v4().to_string();
        let member = Uuid::new_v4().to_string();
        insert_user(&db, &org, UserType::Org).await;
        insert_user(&db, &admin, UserType::Person).await;
        insert_user(&db, &member, UserType::Person).await;
        insert_admin(&db, &org, &admin).await;
        let invite = create_invite(
            &db,
            &org,
            &admin,
            OrgRole::Member,
            MemberScopeSource::Inherit,
            None,
            None,
        )
        .await
        .unwrap();
        db.collection::<bson::Document>(ORG_MEMBERSHIPS)
            .update_one(
                doc! {"org_user_id":&org,"member_user_id":&admin},
                doc! {"$set":{"role":"member"}},
            )
            .await
            .unwrap();
        let rows = db.collection::<bson::Document>(COLLECTION_NAME);
        for change in [
            None,
            Some(doc! {"$unset":{"created_by":""}}),
            Some(doc! {"$set":{"created_by":null}}),
        ] {
            if let Some(update) = change {
                rows.update_one(doc! {"_id":&invite.id}, update)
                    .await
                    .unwrap();
            }
            assert!(matches!(
                redeem_invite(&db, &invite.nonce, &member).await,
                Err(AppError::OrgInviteInvalid(_))
            ));
            assert_eq!(
                rows.count_documents(doc! {"_id":&invite.id,"redeemed_at":null})
                    .await
                    .unwrap(),
                1
            );
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn create_invite_rejects_non_org_owner() {
        let db = connect_test_database("orginv_nonorg")
            .await
            .expect("local MongoDB required for org_invite_service tests");
        let person_id = Uuid::new_v4().to_string();
        insert_user(&db, &person_id, UserType::Person).await;

        let err = create_invite(
            &db,
            &person_id,
            "admin-id",
            OrgRole::Admin,
            MemberScopeSource::Inherit,
            None,
            None,
        )
        .await
        .expect_err("person owner is not an org");
        assert!(matches!(err, AppError::OrgNotFound(id) if id == person_id));
    }

    #[tokio::test]
    async fn redeem_invite_creates_membership_and_marks_invite_redeemed() {
        let db = connect_test_database("orginv_redeem")
            .await
            .expect("local MongoDB required for org_invite_service tests");
        let org_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        insert_user(&db, &org_id, UserType::Org).await;
        insert_user(&db, &member_id, UserType::Person).await;
        insert_user(&db, "admin-id", UserType::Person).await;
        insert_admin(&db, &org_id, "admin-id").await;

        let invite = create_invite(
            &db,
            &org_id,
            "admin-id",
            OrgRole::Viewer,
            MemberScopeSource::Inherit,
            Some(vec!["ignored".to_string()]),
            None,
        )
        .await
        .expect("create invite");

        let membership = redeem_invite(&db, &invite.nonce, &member_id)
            .await
            .expect("redeem invite");
        assert_eq!(membership.org_user_id, org_id);
        assert_eq!(membership.member_user_id, member_id);
        assert_eq!(membership.role, OrgRole::Viewer);
        assert_eq!(membership.scope_source, MemberScopeSource::Inherit);
        assert_eq!(membership.allowed_service_ids, None);
        assert!(membership.is_active());

        let redeemed = db
            .collection::<OrgInvite>(COLLECTION_NAME)
            .find_one(doc! { "_id": &invite.id })
            .await
            .expect("query redeemed invite")
            .expect("redeemed invite exists");
        assert_eq!(redeemed.redeemed_by.as_deref(), Some(member_id.as_str()));
        assert!(redeemed.redeemed_at.is_some());

        let stored_membership = db
            .collection::<crate::models::org_membership::OrgMembership>(ORG_MEMBERSHIPS)
            .find_one(doc! { "_id": &membership.id })
            .await
            .expect("query membership")
            .expect("membership inserted");
        assert_eq!(stored_membership.id, membership.id);
        assert_eq!(stored_membership.org_user_id, membership.org_user_id);
        assert_eq!(stored_membership.member_user_id, membership.member_user_id);
        assert_eq!(stored_membership.role, membership.role);
        assert_eq!(stored_membership.scope_source, membership.scope_source);
        assert_eq!(
            stored_membership.allowed_service_ids,
            membership.allowed_service_ids
        );
        assert_eq!(
            stored_membership.created_at.timestamp_millis(),
            membership.created_at.timestamp_millis()
        );
        assert_eq!(stored_membership.revoked_at, membership.revoked_at);
    }

    #[tokio::test]
    async fn redeem_invite_reports_unknown_expired_and_already_redeemed() {
        let db = connect_test_database("orginv_errs")
            .await
            .expect("local MongoDB required for org_invite_service tests");
        let org_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        let second_member_id = Uuid::new_v4().to_string();
        insert_user(&db, &org_id, UserType::Org).await;
        insert_user(&db, &member_id, UserType::Person).await;
        insert_user(&db, "admin-id", UserType::Person).await;
        insert_admin(&db, &org_id, "admin-id").await;
        insert_user(&db, &second_member_id, UserType::Person).await;

        // A freshly generated invite code that was never persisted -> unknown.
        let missing_nonce = generate_nonce();
        let unknown = redeem_invite(&db, &missing_nonce, &member_id)
            .await
            .expect_err("unknown invite rejected");
        assert!(
            matches!(unknown, AppError::OrgInviteInvalid(message) if message == "unknown invite")
        );

        let expired = create_invite(
            &db,
            &org_id,
            "admin-id",
            OrgRole::Member,
            MemberScopeSource::Override,
            None,
            Some(Duration::seconds(-1)),
        )
        .await
        .expect("create expired invite");
        let expired_err = redeem_invite(&db, &expired.nonce, &member_id)
            .await
            .expect_err("expired invite rejected");
        assert!(matches!(expired_err, AppError::OrgInviteExpired));

        let redeemed = create_invite(
            &db,
            &org_id,
            "admin-id",
            OrgRole::Member,
            MemberScopeSource::Override,
            None,
            None,
        )
        .await
        .expect("create redeemable invite");
        redeem_invite(&db, &redeemed.nonce, &member_id)
            .await
            .expect("first redemption succeeds");
        let redeemed_err = redeem_invite(&db, &redeemed.nonce, &second_member_id)
            .await
            .expect_err("second redemption rejected");
        assert!(matches!(
            redeemed_err,
            AppError::OrgInviteInvalid(message) if message == "invite already redeemed"
        ));
    }

    #[tokio::test]
    async fn cancel_invite_rejects_redeemed_invite() {
        let db = connect_test_database("orginv_cancel")
            .await
            .expect("local MongoDB required for org_invite_service tests");
        let org_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        insert_user(&db, &org_id, UserType::Org).await;
        insert_user(&db, &member_id, UserType::Person).await;
        insert_user(&db, "admin-id", UserType::Person).await;
        insert_admin(&db, &org_id, "admin-id").await;

        let invite = create_invite(
            &db,
            &org_id,
            "admin-id",
            OrgRole::Member,
            MemberScopeSource::Override,
            None,
            None,
        )
        .await
        .expect("create invite");
        redeem_invite(&db, &invite.nonce, &member_id)
            .await
            .expect("redeem invite");

        let err = cancel_invite(&db, &org_id, &invite.id)
            .await
            .expect_err("redeemed invite cannot be cancelled");
        assert!(matches!(
            err,
            AppError::OrgInviteInvalid(message) if message == "cannot cancel a redeemed invite"
        ));
    }

    #[tokio::test]
    async fn redeem_invite_rolls_back_claim_when_membership_creation_fails() {
        let db = connect_test_database("orginv_rb")
            .await
            .expect("local MongoDB required for org_invite_service tests");
        let org_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        insert_user(&db, &org_id, UserType::Org).await;
        insert_user(&db, &member_id, UserType::Person).await;
        insert_user(&db, "admin-id", UserType::Person).await;
        insert_admin(&db, &org_id, "admin-id").await;

        org_service::create_membership(
            &db,
            &org_id,
            &member_id,
            OrgRole::Member,
            MemberScopeSource::Override,
            None,
        )
        .await
        .expect("create existing membership");

        let invite = create_invite(
            &db,
            &org_id,
            "admin-id",
            OrgRole::Admin,
            MemberScopeSource::Override,
            None,
            None,
        )
        .await
        .expect("create invite");
        let err = redeem_invite(&db, &invite.nonce, &member_id)
            .await
            .expect_err("active membership conflicts");
        assert!(
            matches!(err, AppError::Conflict(message) if message == "User is already a member of this org")
        );

        let rolled_back = db
            .collection::<OrgInvite>(COLLECTION_NAME)
            .find_one(doc! { "_id": &invite.id })
            .await
            .expect("query invite")
            .expect("invite remains");
        assert!(rolled_back.redeemed_by.is_none());
        assert!(rolled_back.redeemed_at.is_none());
    }

    #[tokio::test]
    async fn duplicate_key_error_detection_matches_mongo_duplicate_write_error() {
        let db = connect_test_database("orginv_dup")
            .await
            .expect("local MongoDB required for org_invite_service tests");
        let collection = db.collection::<OrgInvite>(COLLECTION_NAME);
        collection
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "nonce": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await
            .expect("create unique nonce index");

        let now = Utc::now();
        let invite = OrgInvite {
            id: Uuid::new_v4().to_string(),
            org_user_id: Uuid::new_v4().to_string(),
            nonce: "ORGINV-DUPLICATEKEYTEST0001".to_string(),
            role: OrgRole::Member,
            scope_source: MemberScopeSource::Override,
            allowed_service_ids: None,
            created_by: "admin-id".to_string(),
            expires_at: now + DEFAULT_INVITE_TTL,
            redeemed_by: None,
            redeemed_at: None,
            created_at: now,
        };
        collection
            .insert_one(&invite)
            .await
            .expect("insert first invite");

        let mut duplicate = invite.clone();
        duplicate.id = Uuid::new_v4().to_string();
        let err = collection
            .insert_one(&duplicate)
            .await
            .expect_err("duplicate nonce rejected by MongoDB");
        assert!(is_duplicate_key_error(&err));
    }
}

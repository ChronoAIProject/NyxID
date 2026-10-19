//! Shared transaction fences for membership mutations and personal departure.
//! The HTTP preflight is advisory; only these guards decide at commit time.
use crate::{
    errors::{AppError, AppResult},
    models::{
        org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership, OrgRole},
        user::{COLLECTION_NAME as USERS, User},
    },
};
use bson::doc;
use mongodb::{ClientSession, Database};

const MAX_DEPARTURE_ORGS: i64 = 256;
const MAX_ADMIN_CANDIDATES: i64 = 256;

fn last_admin() -> AppError {
    AppError::Conflict("Cannot remove or demote the last active admin; promote another active person or delete the organization first".into())
}

pub(crate) async fn fence_org(db: &Database, org: &str, tx: &mut ClientSession) -> AppResult<()> {
    let result = db
        .collection::<bson::Document>(USERS)
        .update_one(
            // Inactive organizations retain membership management/cleanup. Their
            // members must not become unable to leave merely because the org was
            // disabled. Actual organization deletion still contends on this row.
            doc! {"_id":org,"user_type":"org","platform_personal_deleting":{"$ne":true}},
            doc! {"$inc":{"membership_authority_revision":1_i64}},
        )
        .session(tx)
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict(
            "Organization is unavailable or changed; reload membership".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn fence_live_member(
    db: &Database,
    member: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    let result = db.collection::<bson::Document>(USERS).update_one(
        doc! {"_id":member,"user_type":{"$ne":"org"},"is_active":true,"platform_personal_deleting":{"$ne":true}},
        doc! {"$inc":{"membership_authority_revision":1_i64}},
    ).session(tx).await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict(
            "Membership requires an active personal account".into(),
        ));
    }
    Ok(())
}

/// The caller already holds the shared org write fence. Counting membership
/// documents alone would count disabled/deleting people as recovery authority.
pub(crate) async fn ensure_other_admin(
    db: &Database,
    org: &str,
    departing: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    let mut cursor = db.collection::<OrgMembership>(MEMBERSHIPS).find(
        doc! {"org_user_id":org,"member_user_id":{"$ne":departing},"role":"admin","revoked_at":null},
    ).sort(doc! {"member_user_id":1}).limit(MAX_ADMIN_CANDIDATES + 1).session(&mut *tx).await?;
    let mut ids = Vec::new();
    while let Some(row) = cursor.next(&mut *tx).await.transpose()? {
        ids.push(row.member_user_id);
    }
    if !ids.is_empty() && db.collection::<User>(USERS).find_one(doc! {
        "_id":{"$in":&ids},"user_type":{"$ne":"org"},"is_active":true,"platform_personal_deleting":{"$ne":true}
    }).session(&mut *tx).await?.is_some() {
        return Ok(());
    }
    if ids.len() > MAX_ADMIN_CANDIDATES as usize {
        return Err(AppError::Conflict("Organization has too many inactive admin candidates for inline verification; reconcile inactive admin memberships before retrying".into()));
    }
    Err(last_admin())
}

/// Administrative disable is reversible incident containment and must remain
/// available even for a sole admin or an account with many memberships. The
/// person fence serializes registration mint/redemption and membership creation;
/// only permanent departure needs bounded per-org recovery checks.
pub(crate) async fn fence_person_authority_loss(
    db: &Database,
    person: &str,
    tx: &mut ClientSession,
    permanent: bool,
) -> AppResult<()> {
    let target = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id":person})
        .session(&mut *tx)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".into()))?;
    if target.user_type.is_org() {
        return Ok(());
    }
    db.collection::<bson::Document>(USERS)
        .update_one(
            doc! {"_id":person},
            doc! {"$inc":{"membership_authority_revision":1_i64}},
        )
        .session(&mut *tx)
        .await?;
    // Issuer-attributed capabilities can be invalidated directly via indexes.
    // Registration mint/redemption write this same person row, so a held
    // operation cannot install or consume a capability after disable commits.
    // Legacy org tokens without an issuer already require reissuance: they do
    // not acquire authority from this person's still-present membership.
    db.collection::<bson::Document>(crate::models::node_registration_token::COLLECTION_NAME)
        .delete_many(doc! {"used":false,"$or":[{"issuer_user_id":person},{"user_id":person}]})
        .session(&mut *tx)
        .await?;
    if !permanent {
        return Ok(());
    }
    let mut cursor = db
        .collection::<OrgMembership>(MEMBERSHIPS)
        .find(doc! {
            "member_user_id":person,"revoked_at":null
        })
        .sort(doc! {"org_user_id":1})
        .limit(MAX_DEPARTURE_ORGS + 1)
        .session(&mut *tx)
        .await?;
    let mut memberships = Vec::new();
    while let Some(row) = cursor.next(&mut *tx).await.transpose()? {
        memberships.push(row);
    }
    if memberships.len() > MAX_DEPARTURE_ORGS as usize {
        return Err(AppError::Conflict(
            "Remove organization memberships in batches before deleting this account".into(),
        ));
    }
    for membership in memberships {
        // Stale memberships in an already-removed org confer no authority.
        if db
            .collection::<User>(USERS)
            .find_one(doc! {"_id":&membership.org_user_id,"user_type":"org"})
            .session(&mut *tx)
            .await?
            .is_none()
        {
            continue;
        }
        fence_org(db, &membership.org_user_id, &mut *tx).await?;
        if membership.role == OrgRole::Admin {
            ensure_other_admin(db, &membership.org_user_id, person, &mut *tx).await?;
            super::node_service::retire_org_registrations_in_session(
                db,
                &membership.org_user_id,
                person,
                &mut *tx,
            )
            .await?;
        }
    }
    Ok(())
}

/// One transaction owner performs an awaited abort on definite operation error.
/// Commit uncertainty stays with the same-transaction commit helper.
pub(crate) async fn finish<T>(mut tx: ClientSession, result: AppResult<T>) -> AppResult<T> {
    let value = match result {
        Ok(value) => value,
        Err(error) => {
            tx.abort_transaction().await?;
            return Err(match error {
                AppError::DatabaseError(error) => super::transaction_service::map_error(error),
                error => error,
            });
        }
    };
    super::transaction_service::commit(&mut tx).await?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::super::org_service;
    use super::*;
    use crate::{
        models::{org_membership::MemberScopeSource, user::UserType},
        test_utils::{connect_test_database, test_user},
    };

    async fn fixture() -> (Database, String, [String; 2]) {
        let db = connect_test_database("personal_erasure_org_fence")
            .await
            .expect("isolated MongoDB required");
        let org = uuid::Uuid::new_v4().to_string();
        let people = [
            uuid::Uuid::new_v4().to_string(),
            uuid::Uuid::new_v4().to_string(),
        ];
        db.collection::<User>(USERS)
            .insert_one(test_user(&org, UserType::Org))
            .await
            .unwrap();
        for person in &people {
            db.collection::<User>(USERS)
                .insert_one(test_user(person, UserType::Person))
                .await
                .unwrap();
            org_service::create_membership(
                &db,
                &org,
                person,
                OrgRole::Admin,
                MemberScopeSource::Inherit,
                None,
            )
            .await
            .unwrap();
        }
        (db, org, people)
    }

    async fn depart(db: &Database, person: &str) -> AppResult<()> {
        let mut tx = db.client().start_session().await?;
        tx.start_transaction()
            .write_concern(mongodb::options::WriteConcern::majority())
            .await?;
        let result: AppResult<()> = async {
            fence_person_authority_loss(db, person, &mut tx, true).await?;
            db.collection::<bson::Document>(USERS)
                .update_one(
                    doc! {"_id":person},
                    doc! {"$set":{"is_active":false,"platform_personal_deleting":true}},
                )
                .session(&mut tx)
                .await?;
            Ok(())
        }
        .await;
        finish(tx, result).await
    }

    #[tokio::test]
    async fn concurrent_person_departures_preserve_one_live_admin_across_clients() {
        let (db, org, people) = fixture().await;
        let independent =
            mongodb::Client::with_uri_str(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
                .await
                .unwrap()
                .database(db.name());
        let (left, right) = tokio::join!(depart(&db, &people[0]), depart(&independent, &people[1]));
        assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
        for result in [left, right] {
            if let Err(error) = result {
                assert!(matches!(error, AppError::Conflict(_)));
            }
        }
        assert_eq!(
            db.collection::<User>(USERS)
                .count_documents(doc! {"_id":{"$in":people.to_vec()},"is_active":true})
                .await
                .unwrap(),
            1
        );
        assert!(
            db.collection::<User>(USERS)
                .find_one(doc! {"_id":&org})
                .await
                .unwrap()
                .is_some()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn membership_mutation_and_departure_share_guard_and_rejection_preserves_account() {
        let (db, org, people) = fixture().await;
        let other = org_service::get_active_membership(&db, &org, &people[1])
            .await
            .unwrap()
            .unwrap();
        org_service::update_membership(&db, &other.id, Some(OrgRole::Member), None, None)
            .await
            .unwrap();
        assert!(matches!(
            depart(&db, &people[0]).await,
            Err(AppError::Conflict(_))
        ));
        assert!(db.collection::<User>(USERS).find_one(doc! {"_id":&people[0],"is_active":true,"platform_personal_deleting":{"$ne":true}}).await.unwrap().is_some());
        assert!(matches!(
            org_service::revoke_membership(&db, &org, &people[0]).await,
            Err(AppError::Conflict(_))
        ));
        let first = org_service::get_active_membership(&db, &org, &people[0])
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            org_service::update_membership(&db, &first.id, Some(OrgRole::Viewer), None, None).await,
            Err(AppError::Conflict(_))
        ));
        org_service::update_membership(&db, &other.id, Some(OrgRole::Admin), None, None)
            .await
            .unwrap();
        let independent =
            mongodb::Client::with_uri_str(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
                .await
                .unwrap()
                .database(db.name());
        let (left, right) = tokio::join!(
            depart(&db, &people[0]),
            org_service::revoke_membership(&independent, &org, &people[1])
        );
        assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
        for result in [left, right] {
            if let Err(error) = result {
                assert!(matches!(error, AppError::Conflict(_)));
            }
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn inactive_org_allows_ordinary_member_departure_without_destroying_org() {
        let (db, org, people) = fixture().await;
        let membership = org_service::get_active_membership(&db, &org, &people[1])
            .await
            .unwrap()
            .unwrap();
        org_service::update_membership(&db, &membership.id, Some(OrgRole::Member), None, None)
            .await
            .unwrap();
        db.collection::<User>(USERS)
            .update_one(doc! {"_id":&org}, doc! {"$set":{"is_active":false}})
            .await
            .unwrap();
        org_service::revoke_membership(&db, &org, &people[1])
            .await
            .unwrap();
        depart(&db, &people[1]).await.unwrap();
        assert!(
            db.collection::<User>(USERS)
                .find_one(doc! {"_id":&org,"is_active":false})
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            db.collection::<User>(USERS)
                .find_one(doc! {"_id":&people[0],"is_active":true})
                .await
                .unwrap()
                .is_some()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn admin_disable_contains_sole_admin_without_permanent_departure_veto() {
        let (db, org, people) = fixture().await;
        let operator = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&operator, UserType::Person))
            .await
            .unwrap();
        let other = org_service::get_active_membership(&db, &org, &people[1])
            .await
            .unwrap()
            .unwrap();
        org_service::update_membership(&db, &other.id, Some(OrgRole::Member), None, None)
            .await
            .unwrap();
        let (_, pending, _) = crate::services::node_service::create_registration_token_for_actor(
            &db,
            &people[0],
            &org,
            "incident-token",
            10,
            300,
        )
        .await
        .unwrap();
        crate::services::admin_user_service::set_user_active(&db, &operator, &people[0], false)
            .await
            .unwrap();
        assert!(
            db.collection::<User>(USERS)
                .find_one(doc! {"_id":&org,"is_active":true})
                .await
                .unwrap()
                .is_some()
        );
        assert!(db.collection::<User>(USERS).find_one(doc! {"_id":&people[0],"is_active":false,"platform_personal_deleting":{"$ne":true}}).await.unwrap().is_some());
        assert!(
            crate::services::node_service::register_node(
                &db,
                &crate::test_utils::test_encryption_keys(),
                &pending,
                None
            )
            .await
            .is_err()
        );
        crate::services::admin_user_service::set_user_active(&db, &operator, &people[0], true)
            .await
            .unwrap();
        assert!(
            db.collection::<User>(USERS)
                .find_one(doc! {"_id":&people[0],"is_active":true})
                .await
                .unwrap()
                .is_some()
        );
        assert!(matches!(
            depart(&db, &people[0]).await,
            Err(AppError::Conflict(_))
        ));
        org_service::update_membership(&db, &other.id, Some(OrgRole::Admin), None, None)
            .await
            .unwrap();
        crate::services::admin_user_service::set_user_active(&db, &operator, &people[1], false)
            .await
            .unwrap();
        // A disabled membership cannot satisfy permanent recovery requirements.
        assert!(matches!(
            depart(&db, &people[0]).await,
            Err(AppError::Conflict(_))
        ));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn emergency_disable_has_no_membership_cardinality_veto() {
        let (db, org, people) = fixture().await;
        let template = org_service::get_active_membership(&db, &org, &people[0])
            .await
            .unwrap()
            .unwrap();
        let mut memberships = Vec::new();
        let mut orgs = Vec::new();
        for _ in 0..=MAX_DEPARTURE_ORGS {
            let id = uuid::Uuid::new_v4().to_string();
            let mut membership = template.clone();
            membership.id = uuid::Uuid::new_v4().to_string();
            membership.org_user_id = id.clone();
            memberships.push(membership);
            orgs.push(test_user(&id, UserType::Org));
        }
        db.collection::<User>(USERS)
            .insert_many(orgs)
            .await
            .unwrap();
        db.collection::<OrgMembership>(MEMBERSHIPS)
            .insert_many(memberships)
            .await
            .unwrap();
        let (_, pending, _) = crate::services::node_service::create_registration_token_for_actor(
            &db,
            &people[0],
            &org,
            "many-org-incident-token",
            10,
            300,
        )
        .await
        .unwrap();
        crate::services::admin_user_service::set_user_active(&db, &people[1], &people[0], false)
            .await
            .unwrap();
        assert!(
            db.collection::<User>(USERS)
                .find_one(doc! {"_id":&people[0],"is_active":false})
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(
            db.collection::<User>(USERS)
                .count_documents(doc! {"user_type":"org"})
                .await
                .unwrap(),
            (MAX_DEPARTURE_ORGS + 2) as u64
        );
        assert!(
            crate::services::node_service::register_node(
                &db,
                &crate::test_utils::test_encryption_keys(),
                &pending,
                None,
            )
            .await
            .is_err()
        );
        crate::services::admin_user_service::set_user_active(&db, &people[1], &people[0], true)
            .await
            .unwrap();
        // Permanent erasure retains its actionable bounded recovery preflight.
        assert!(matches!(depart(&db, &people[0]).await,
            Err(AppError::Conflict(message)) if message.contains("memberships in batches")));
        assert!(
            db.collection::<User>(USERS)
                .find_one(doc! {"_id":&people[0],"is_active":true})
                .await
                .unwrap()
                .is_some()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn live_257th_admin_is_counted_and_exhausted_inline_bound_is_actionable() {
        let (db, org, people) = fixture().await;
        let other = org_service::get_active_membership(&db, &org, &people[1])
            .await
            .unwrap()
            .unwrap();
        org_service::update_membership(&db, &other.id, Some(OrgRole::Member), None, None)
            .await
            .unwrap();
        let template = org_service::get_active_membership(&db, &org, &people[0])
            .await
            .unwrap()
            .unwrap();
        let mut memberships = Vec::new();
        for index in 0..=MAX_ADMIN_CANDIDATES {
            let mut membership = template.clone();
            membership.id = uuid::Uuid::new_v4().to_string();
            membership.member_user_id = format!("00000000-0000-4000-8000-{index:012}");
            memberships.push(membership);
        }
        let live = memberships.last().unwrap().member_user_id.clone();
        db.collection::<OrgMembership>(MEMBERSHIPS)
            .insert_many(memberships)
            .await
            .unwrap();
        db.collection::<User>(USERS)
            .insert_one(test_user(&live, UserType::Person))
            .await
            .unwrap();
        let mut tx = db.client().start_session().await.unwrap();
        tx.start_transaction().await.unwrap();
        fence_org(&db, &org, &mut tx).await.unwrap();
        ensure_other_admin(&db, &org, &people[0], &mut tx)
            .await
            .unwrap();
        tx.abort_transaction().await.unwrap();
        db.collection::<User>(USERS)
            .update_one(doc! {"_id":&live}, doc! {"$set":{"is_active":false}})
            .await
            .unwrap();
        let error = depart(&db, &people[0]).await.unwrap_err();
        assert!(
            matches!(error, AppError::Conflict(message) if message.contains("reconcile inactive admin memberships"))
        );
        assert!(
            db.collection::<User>(USERS)
                .find_one(doc! {"_id":&people[0],"is_active":true})
                .await
                .unwrap()
                .is_some()
        );
        db.drop().await.unwrap();
    }
}

use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};
use uuid::Uuid;

use crate::errors::AppResult;
use crate::models::org_membership::{MemberScopeSource, OrgMembership, OrgRole};
use crate::models::org_role_scope::{COLLECTION_NAME, OrgRoleScope};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrgRoleScopeListItem {
    pub role: OrgRole,
    pub allowed_service_ids: Option<Vec<String>>,
    pub is_default: bool,
    pub updated_at: Option<DateTime<Utc>>,
    pub updated_by: Option<String>,
}

pub async fn get_scope(
    db: &mongodb::Database,
    org_user_id: &str,
    role: OrgRole,
) -> AppResult<Option<OrgRoleScope>> {
    let scope = db
        .collection::<OrgRoleScope>(COLLECTION_NAME)
        .find_one(doc! { "org_user_id": org_user_id, "role": role.as_str() })
        .await?;
    Ok(scope)
}

/// Return one effective row per org role.
///
/// Missing stored rows are synthesized as full-access defaults
/// (`allowed_service_ids = None`) and marked with `is_default = true`.
pub async fn list_scopes(
    db: &mongodb::Database,
    org_user_id: &str,
) -> AppResult<Vec<OrgRoleScopeListItem>> {
    let stored: Vec<OrgRoleScope> = db
        .collection::<OrgRoleScope>(COLLECTION_NAME)
        .find(doc! { "org_user_id": org_user_id })
        .await?
        .try_collect()
        .await?;

    let mut by_role = std::collections::HashMap::new();
    for scope in stored {
        by_role.insert(scope.role, scope);
    }

    Ok(OrgRole::ALL
        .into_iter()
        .map(|role| match by_role.remove(&role) {
            Some(scope) => OrgRoleScopeListItem {
                role,
                allowed_service_ids: scope.allowed_service_ids,
                is_default: false,
                updated_at: Some(scope.updated_at),
                updated_by: Some(scope.updated_by),
            },
            None => OrgRoleScopeListItem {
                role,
                allowed_service_ids: None,
                is_default: true,
                updated_at: None,
                updated_by: None,
            },
        })
        .collect())
}

pub async fn set_scope(
    db: &mongodb::Database,
    org_user_id: &str,
    role: OrgRole,
    allowed_service_ids: Option<Vec<String>>,
    actor_id: &str,
) -> AppResult<OrgRoleScope> {
    let now = Utc::now();
    let now_bson = bson::DateTime::from_chrono(now);
    let allowed = match &allowed_service_ids {
        None => bson::Bson::Null,
        Some(ids) => {
            bson::to_bson(ids).map_err(|e| crate::errors::AppError::Internal(e.to_string()))?
        }
    };

    let scope = db
        .collection::<OrgRoleScope>(COLLECTION_NAME)
        .find_one_and_update(
            doc! { "org_user_id": org_user_id, "role": role.as_str() },
            doc! {
                "$set": {
                    "allowed_service_ids": allowed,
                    "updated_at": now_bson,
                    "updated_by": actor_id,
                },
                "$setOnInsert": {
                    "_id": Uuid::new_v4().to_string(),
                    "org_user_id": org_user_id,
                    "role": role.as_str(),
                },
            },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .upsert(true)
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?
        .ok_or_else(|| {
            crate::errors::AppError::Internal(
                "Role scope upsert did not return the updated row".to_string(),
            )
        })?;

    Ok(scope)
}

pub async fn clear_scope(
    db: &mongodb::Database,
    org_user_id: &str,
    role: OrgRole,
) -> AppResult<()> {
    db.collection::<OrgRoleScope>(COLLECTION_NAME)
        .delete_one(doc! { "org_user_id": org_user_id, "role": role.as_str() })
        .await?;
    Ok(())
}

/// Resolve the service scope a membership should enforce right now.
///
/// `Override` returns the membership row's explicit scope. `Inherit` reads
/// the current scope for the membership's role. A missing role-scope row is
/// the default full-access state, represented by `None`.
pub async fn effective_scope_for_membership(
    db: &mongodb::Database,
    membership: &OrgMembership,
) -> AppResult<Option<Vec<String>>> {
    match membership.scope_source {
        MemberScopeSource::Override => Ok(membership.allowed_service_ids.clone()),
        MemberScopeSource::Inherit => Ok(get_scope(db, &membership.org_user_id, membership.role)
            .await?
            .and_then(|scope| scope.allowed_service_ids)),
    }
}

pub fn scope_allows(effective: &Option<Vec<String>>, svc_id: &str) -> bool {
    match effective {
        None => true,
        Some(ids) => ids.iter().any(|id| id == svc_id),
    }
}

pub async fn delete_all_for_org(db: &mongodb::Database, org_user_id: &str) -> AppResult<()> {
    db.collection::<OrgRoleScope>(COLLECTION_NAME)
        .delete_many(doc! { "org_user_id": org_user_id })
        .await?;
    Ok(())
}

pub async fn remove_service_from_all_scopes(
    db: &mongodb::Database,
    org_user_id: &str,
    service_id: &str,
) -> AppResult<()> {
    db.collection::<OrgRoleScope>(COLLECTION_NAME)
        .update_many(
            doc! {
                "org_user_id": org_user_id,
                "allowed_service_ids": service_id,
            },
            doc! { "$pull": { "allowed_service_ids": service_id } },
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::org_role_scope::COLLECTION_NAME as ORG_ROLE_SCOPES;
    use crate::test_utils::{connect_test_database, test_membership};

    #[test]
    fn scope_allows_cases() {
        assert!(scope_allows(&None, "svc-1"));
        assert!(scope_allows(&Some(vec!["svc-1".to_string()]), "svc-1"));
        assert!(!scope_allows(&Some(vec!["svc-2".to_string()]), "svc-1"));
        assert!(!scope_allows(&Some(vec![]), "svc-1"));
    }

    #[tokio::test]
    async fn set_get_list_and_clear_scope() {
        let Some(db) = connect_test_database("org_role_scope_crud").await else {
            eprintln!("skipping org role scope service test: no local MongoDB available");
            return;
        };
        let org_id = Uuid::new_v4().to_string();
        let actor_id = Uuid::new_v4().to_string();

        let stored = set_scope(
            &db,
            &org_id,
            OrgRole::Member,
            Some(vec!["svc-1".to_string(), "svc-2".to_string()]),
            &actor_id,
        )
        .await
        .expect("set role scope");
        assert_eq!(stored.org_user_id, org_id);
        assert_eq!(stored.role, OrgRole::Member);
        assert_eq!(
            stored.allowed_service_ids,
            Some(vec!["svc-1".to_string(), "svc-2".to_string()])
        );
        assert_eq!(stored.updated_by, actor_id);

        let fetched = get_scope(&db, &org_id, OrgRole::Member)
            .await
            .expect("get scope")
            .expect("stored scope");
        assert_eq!(fetched.id, stored.id);

        let listed = list_scopes(&db, &org_id).await.expect("list scopes");
        assert_eq!(listed.len(), 3);
        let admin = listed.iter().find(|s| s.role == OrgRole::Admin).unwrap();
        let member = listed.iter().find(|s| s.role == OrgRole::Member).unwrap();
        assert!(admin.is_default);
        assert!(!member.is_default);
        assert_eq!(
            member.allowed_service_ids,
            Some(vec!["svc-1".to_string(), "svc-2".to_string()])
        );

        clear_scope(&db, &org_id, OrgRole::Member)
            .await
            .expect("clear scope");
        let cleared = get_scope(&db, &org_id, OrgRole::Member)
            .await
            .expect("get cleared");
        assert!(cleared.is_none());
    }

    #[tokio::test]
    async fn effective_scope_respects_override_and_inherit_modes() {
        let Some(db) = connect_test_database("org_role_scope_effective").await else {
            eprintln!("skipping org role scope service test: no local MongoDB available");
            return;
        };
        let org_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        let actor_id = Uuid::new_v4().to_string();

        set_scope(
            &db,
            &org_id,
            OrgRole::Member,
            Some(vec!["role-svc".to_string()]),
            &actor_id,
        )
        .await
        .expect("set role scope");

        let mut override_membership = test_membership(
            &org_id,
            &member_id,
            OrgRole::Member,
            Some(vec!["override-svc".to_string()]),
        );
        override_membership.scope_source = MemberScopeSource::Override;
        assert_eq!(
            effective_scope_for_membership(&db, &override_membership)
                .await
                .expect("override effective"),
            Some(vec!["override-svc".to_string()])
        );

        let mut inherited = override_membership.clone();
        inherited.scope_source = MemberScopeSource::Inherit;
        assert_eq!(
            effective_scope_for_membership(&db, &inherited)
                .await
                .expect("inherit effective"),
            Some(vec!["role-svc".to_string()])
        );

        let mut viewer = inherited;
        viewer.role = OrgRole::Viewer;
        assert_eq!(
            effective_scope_for_membership(&db, &viewer)
                .await
                .expect("missing role scope is full access"),
            None
        );
    }

    #[tokio::test]
    async fn delete_all_and_remove_service_cleanup() {
        let Some(db) = connect_test_database("org_role_scope_cleanup").await else {
            eprintln!("skipping org role scope service test: no local MongoDB available");
            return;
        };
        let org_id = Uuid::new_v4().to_string();
        let other_org_id = Uuid::new_v4().to_string();
        let actor_id = Uuid::new_v4().to_string();

        set_scope(
            &db,
            &org_id,
            OrgRole::Admin,
            Some(vec!["svc-1".to_string(), "svc-2".to_string()]),
            &actor_id,
        )
        .await
        .unwrap();
        set_scope(
            &db,
            &org_id,
            OrgRole::Member,
            Some(vec!["svc-1".to_string()]),
            &actor_id,
        )
        .await
        .unwrap();
        set_scope(
            &db,
            &other_org_id,
            OrgRole::Member,
            Some(vec!["svc-1".to_string()]),
            &actor_id,
        )
        .await
        .unwrap();

        remove_service_from_all_scopes(&db, &org_id, "svc-1")
            .await
            .expect("remove service");
        let admin = get_scope(&db, &org_id, OrgRole::Admin)
            .await
            .unwrap()
            .unwrap();
        let member = get_scope(&db, &org_id, OrgRole::Member)
            .await
            .unwrap()
            .unwrap();
        let other = get_scope(&db, &other_org_id, OrgRole::Member)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(admin.allowed_service_ids, Some(vec!["svc-2".to_string()]));
        assert_eq!(member.allowed_service_ids, Some(vec![]));
        assert_eq!(other.allowed_service_ids, Some(vec!["svc-1".to_string()]));

        delete_all_for_org(&db, &org_id).await.expect("delete all");
        let remaining = db
            .collection::<OrgRoleScope>(ORG_ROLE_SCOPES)
            .count_documents(doc! { "org_user_id": &org_id })
            .await
            .unwrap();
        assert_eq!(remaining, 0);
    }
}

//! Per-org feature flags.
//!
//! Flags are **declared in code** (the [`FEATURE_FLAGS`] registry is the single
//! source of truth) and **toggled at runtime** per org via override rows stored
//! in `feature_flag_overrides`. Resolution is server-authoritative: the backend
//! computes the effective enabled set for a given (org, member, role) and the
//! client only receives the resolved list — it never sees the override rules.
//!
//! Precedence, most-specific first:
//! `user override` → `role override` → `org override` → `code default`.
//!
//! Adding a new flag = add a [`FeatureFlagDef`] entry here (ships with a deploy)
//! and consume its key on the frontend. Toggling an existing flag for an org /
//! role / user is a runtime write, no deploy. Mirror new keys in
//! `frontend/src/lib/feature-flags.ts`.

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::feature_flag_override::{COLLECTION_NAME, FeatureFlagOverride, FlagTargetKind};
use crate::models::org_membership::OrgRole;

// ─────────────────────────────────────────────────────────────────────────────
// Registry (source of truth — declared in code)
// ─────────────────────────────────────────────────────────────────────────────

/// A feature flag definition. `default_enabled` is the value used when no
/// override applies; `org_manageable` gates whether an org admin may toggle it
/// (set `false` for staff-only flags kept out of the org self-serve surface).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FeatureFlagDef {
    pub key: &'static str,
    pub description: &'static str,
    pub default_enabled: bool,
    pub org_manageable: bool,
}

/// The known feature flags. Keys are stable identifiers referenced by both the
/// backend and `frontend/src/lib/feature-flags.ts`.
///
/// No flags are shipped yet — the registry is intentionally empty so the
/// product surfaces render their empty states. Add real flags here (and mirror
/// the key in `frontend/src/lib/feature-flags.ts`) when introducing one.
#[cfg(not(test))]
pub const FEATURE_FLAGS: &[FeatureFlagDef] = &[];

/// Test builds carry a placeholder flag so the resolution / override pipeline
/// stays exercised even while the production registry is empty.
#[cfg(test)]
pub const FEATURE_FLAGS: &[FeatureFlagDef] = &[FeatureFlagDef {
    key: "example_ui",
    description: "Test-only placeholder flag.",
    default_enabled: false,
    org_manageable: true,
}];

/// Look up a flag definition by key. `None` for unknown keys.
pub fn find_flag(key: &str) -> Option<&'static FeatureFlagDef> {
    FEATURE_FLAGS.iter().find(|f| f.key == key)
}

// ─────────────────────────────────────────────────────────────────────────────
// Target
// ─────────────────────────────────────────────────────────────────────────────

/// Ergonomic view of an override's scope, used by the service and handlers.
/// Maps to the flat `(target_kind, target_key)` columns on the stored row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlagTarget {
    /// Platform-wide (everyone, org or not). A baseline below org scopes.
    Global,
    /// Every member of the org.
    Org,
    /// Members holding this role.
    Role(OrgRole),
    /// A single user. Their `member_user_id` (org context) or user id (personal).
    User(String),
}

impl FlagTarget {
    pub fn kind(&self) -> FlagTargetKind {
        match self {
            Self::Global => FlagTargetKind::Global,
            Self::Org => FlagTargetKind::Org,
            Self::Role(_) => FlagTargetKind::Role,
            Self::User(_) => FlagTargetKind::User,
        }
    }

    /// Storage key: `None` for global/org scope, role string for role scope,
    /// user id for user scope.
    pub fn key(&self) -> Option<String> {
        match self {
            Self::Global | Self::Org => None,
            Self::Role(role) => Some(role.as_str().to_string()),
            Self::User(user_id) => Some(user_id.clone()),
        }
    }

    /// Parse a wire target (`kind` + optional `value`) into a `FlagTarget`,
    /// validating that role targets name a real role and that a value is
    /// present/absent as the kind requires.
    pub fn from_parts(kind: FlagTargetKind, value: Option<&str>) -> AppResult<Self> {
        match kind {
            FlagTargetKind::Global => Ok(Self::Global),
            FlagTargetKind::Org => Ok(Self::Org),
            FlagTargetKind::Role => {
                let raw = value.ok_or_else(|| {
                    AppError::BadRequest("role target requires a target_value".to_string())
                })?;
                let role = match raw {
                    "admin" => OrgRole::Admin,
                    "member" => OrgRole::Member,
                    "viewer" => OrgRole::Viewer,
                    other => {
                        return Err(AppError::BadRequest(format!(
                            "invalid org role '{other}'; expected admin, member, or viewer"
                        )));
                    }
                };
                Ok(Self::Role(role))
            }
            FlagTargetKind::User => {
                let user_id = value.ok_or_else(|| {
                    AppError::BadRequest("user target requires a target_value".to_string())
                })?;
                if user_id.trim().is_empty() {
                    return Err(AppError::BadRequest(
                        "user target_value must not be empty".to_string(),
                    ));
                }
                Ok(Self::User(user_id.to_string()))
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Resolution
// ─────────────────────────────────────────────────────────────────────────────

/// Pick the enabled value of the override matching `(flag_key, kind, key)`.
fn pick_override(
    overrides: &[FeatureFlagOverride],
    flag_key: &str,
    kind: FlagTargetKind,
    key: Option<&str>,
) -> Option<bool> {
    overrides
        .iter()
        .find(|o| o.flag_key == flag_key && o.target_kind == kind && o.target_key.as_deref() == key)
        .map(|o| o.enabled)
}

/// Compute the enabled-flag keys for a member in an org context.
///
/// Pure and DB-free so precedence is unit-testable. Per flag, most-specific
/// wins: `default → global → org → role(matching `role`) → user(matching
/// `member_user_id`)`. `global` rows come from the platform-level set; the org,
/// role, and user rows from the org's set.
pub fn resolve_from_overrides(
    global: &[FeatureFlagOverride],
    org: &[FeatureFlagOverride],
    member_user_id: &str,
    role: OrgRole,
) -> Vec<String> {
    let mut enabled_keys = Vec::new();
    for def in FEATURE_FLAGS {
        let mut enabled = def.default_enabled;
        if let Some(v) = pick_override(global, def.key, FlagTargetKind::Global, None) {
            enabled = v;
        }
        if let Some(v) = pick_override(org, def.key, FlagTargetKind::Org, None) {
            enabled = v;
        }
        if let Some(v) = pick_override(org, def.key, FlagTargetKind::Role, Some(role.as_str())) {
            enabled = v;
        }
        if let Some(v) = pick_override(org, def.key, FlagTargetKind::User, Some(member_user_id)) {
            enabled = v;
        }
        if enabled {
            enabled_keys.push(def.key.to_string());
        }
    }
    enabled_keys
}

/// Compute the enabled-flag keys for a user in the **personal** (non-org)
/// context. Per flag: `default → global → user(personal, matching `user_id`)`.
///
/// This is what a user with zero orgs — or any non-org surface — sees. No org
/// rungs apply.
pub fn resolve_personal_from_overrides(
    platform: &[FeatureFlagOverride],
    user_id: &str,
) -> Vec<String> {
    let mut enabled_keys = Vec::new();
    for def in FEATURE_FLAGS {
        let mut enabled = def.default_enabled;
        if let Some(v) = pick_override(platform, def.key, FlagTargetKind::Global, None) {
            enabled = v;
        }
        if let Some(v) = pick_override(platform, def.key, FlagTargetKind::User, Some(user_id)) {
            enabled = v;
        }
        if enabled {
            enabled_keys.push(def.key.to_string());
        }
    }
    enabled_keys
}

/// Resolve enabled-flag keys for a member of an org (org context). Applies the
/// platform-global baseline plus the org's own overrides.
pub async fn resolve_enabled_features(
    db: &mongodb::Database,
    org_user_id: &str,
    member_user_id: &str,
    role: OrgRole,
) -> AppResult<Vec<String>> {
    let org = list_overrides(db, org_user_id).await?;
    let global = list_platform_overrides(db).await?;
    Ok(resolve_from_overrides(&global, &org, member_user_id, role))
}

/// Resolve enabled-flag keys for a user in the personal (non-org) context.
/// Delivered on `GET /users/me` so users with zero orgs still get their flags.
pub async fn resolve_personal_features(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<Vec<String>> {
    let platform = list_platform_overrides(db).await?;
    Ok(resolve_personal_from_overrides(&platform, user_id))
}

// ─────────────────────────────────────────────────────────────────────────────
// Management (org-admin self-serve)
// ─────────────────────────────────────────────────────────────────────────────

/// All override rows for an org.
pub async fn list_overrides(
    db: &mongodb::Database,
    org_user_id: &str,
) -> AppResult<Vec<FeatureFlagOverride>> {
    let rows: Vec<FeatureFlagOverride> = db
        .collection::<FeatureFlagOverride>(COLLECTION_NAME)
        .find(doc! { "org_user_id": org_user_id })
        .await?
        .try_collect()
        .await?;
    Ok(rows)
}

/// All platform-level override rows: global rollout + personal (org-less) user
/// overrides. These have `org_user_id` unset (`null`), so an org-scoped
/// `list_overrides` never returns them and vice versa.
pub async fn list_platform_overrides(
    db: &mongodb::Database,
) -> AppResult<Vec<FeatureFlagOverride>> {
    let rows: Vec<FeatureFlagOverride> = db
        .collection::<FeatureFlagOverride>(COLLECTION_NAME)
        .find(doc! { "org_user_id": bson::Bson::Null })
        .await?
        .try_collect()
        .await?;
    Ok(rows)
}

/// A registry flag paired with the org's current overrides for it. Powers the
/// admin toggle UI.
#[derive(Clone, Debug)]
pub struct FeatureFlagAdminItem {
    pub def: &'static FeatureFlagDef,
    pub overrides: Vec<FeatureFlagOverride>,
}

/// List every registry flag with the org's overrides attached.
pub async fn list_for_admin(
    db: &mongodb::Database,
    org_user_id: &str,
) -> AppResult<Vec<FeatureFlagAdminItem>> {
    let all = list_overrides(db, org_user_id).await?;
    Ok(FEATURE_FLAGS
        .iter()
        .map(|def| FeatureFlagAdminItem {
            def,
            overrides: all
                .iter()
                .filter(|o| o.flag_key == def.key)
                .cloned()
                .collect(),
        })
        .collect())
}

/// Upsert an override. Rejects unknown flag keys (400) and non-org-manageable
/// flags (403). Idempotent per `(org, flag, target_kind, target_key)`.
pub async fn set_override(
    db: &mongodb::Database,
    org_user_id: &str,
    flag_key: &str,
    target: &FlagTarget,
    enabled: bool,
    actor_id: &str,
) -> AppResult<FeatureFlagOverride> {
    let def = find_flag(flag_key)
        .ok_or_else(|| AppError::BadRequest(format!("unknown feature flag '{flag_key}'")))?;
    if !def.org_manageable {
        return Err(AppError::Forbidden(format!(
            "feature flag '{flag_key}' is not org-manageable"
        )));
    }

    let now = bson::DateTime::from_chrono(Utc::now());
    let target_key = match target.key() {
        Some(k) => bson::Bson::String(k),
        None => bson::Bson::Null,
    };

    let row = db
        .collection::<FeatureFlagOverride>(COLLECTION_NAME)
        .find_one_and_update(
            doc! {
                "org_user_id": org_user_id,
                "flag_key": flag_key,
                "target_kind": target.kind().as_str(),
                "target_key": target_key.clone(),
            },
            doc! {
                "$set": {
                    "enabled": enabled,
                    "updated_at": now,
                    "updated_by": actor_id,
                },
                "$setOnInsert": {
                    "_id": Uuid::new_v4().to_string(),
                    "org_user_id": org_user_id,
                    "flag_key": flag_key,
                    "target_kind": target.kind().as_str(),
                    "target_key": target_key,
                    "created_at": now,
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
            AppError::Internal("feature flag override upsert did not return the row".to_string())
        })?;

    Ok(row)
}

/// Delete one override. Returns whether a row was removed (idempotent clear).
pub async fn clear_override(
    db: &mongodb::Database,
    org_user_id: &str,
    flag_key: &str,
    target: &FlagTarget,
) -> AppResult<bool> {
    let target_key = match target.key() {
        Some(k) => bson::Bson::String(k),
        None => bson::Bson::Null,
    };
    let res = db
        .collection::<FeatureFlagOverride>(COLLECTION_NAME)
        .delete_one(doc! {
            "org_user_id": org_user_id,
            "flag_key": flag_key,
            "target_kind": target.kind().as_str(),
            "target_key": target_key,
        })
        .await?;
    Ok(res.deleted_count > 0)
}

/// Upsert a platform-level override (staff-controlled). Only `Global` and
/// personal `User` targets are valid — org/role scopes require an org, and are
/// rejected here. No `org_manageable` gate: platform admins own these.
///
// Write path for the forthcoming platform-admin `/admin/feature-flags` surface;
// exercised by tests today. `allow(dead_code)` until those endpoints land.
#[allow(dead_code)]
pub async fn set_platform_override(
    db: &mongodb::Database,
    flag_key: &str,
    target: &FlagTarget,
    enabled: bool,
    actor_id: &str,
) -> AppResult<FeatureFlagOverride> {
    find_flag(flag_key)
        .ok_or_else(|| AppError::BadRequest(format!("unknown feature flag '{flag_key}'")))?;
    if matches!(target, FlagTarget::Org | FlagTarget::Role(_)) {
        return Err(AppError::BadRequest(
            "org and role targets require an org; use the org feature-flag API".to_string(),
        ));
    }

    let now = bson::DateTime::from_chrono(Utc::now());
    let target_key = match target.key() {
        Some(k) => bson::Bson::String(k),
        None => bson::Bson::Null,
    };

    let row = db
        .collection::<FeatureFlagOverride>(COLLECTION_NAME)
        .find_one_and_update(
            doc! {
                "org_user_id": bson::Bson::Null,
                "flag_key": flag_key,
                "target_kind": target.kind().as_str(),
                "target_key": target_key.clone(),
            },
            doc! {
                "$set": {
                    "enabled": enabled,
                    "updated_at": now,
                    "updated_by": actor_id,
                },
                "$setOnInsert": {
                    "_id": Uuid::new_v4().to_string(),
                    "flag_key": flag_key,
                    "target_kind": target.kind().as_str(),
                    "target_key": target_key,
                    "created_at": now,
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
            AppError::Internal(
                "platform feature flag override upsert did not return the row".to_string(),
            )
        })?;
    Ok(row)
}

/// Clear a platform-level override. Returns whether a row was removed.
#[allow(dead_code)]
pub async fn clear_platform_override(
    db: &mongodb::Database,
    flag_key: &str,
    target: &FlagTarget,
) -> AppResult<bool> {
    let target_key = match target.key() {
        Some(k) => bson::Bson::String(k),
        None => bson::Bson::Null,
    };
    let res = db
        .collection::<FeatureFlagOverride>(COLLECTION_NAME)
        .delete_one(doc! {
            "org_user_id": bson::Bson::Null,
            "flag_key": flag_key,
            "target_kind": target.kind().as_str(),
            "target_key": target_key,
        })
        .await?;
    Ok(res.deleted_count > 0)
}

/// Cascade helper: drop every override for an org (used when the org is deleted).
pub async fn delete_all_for_org(db: &mongodb::Database, org_user_id: &str) -> AppResult<()> {
    db.collection::<FeatureFlagOverride>(COLLECTION_NAME)
        .delete_many(doc! { "org_user_id": org_user_id })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::connect_test_database;

    fn override_row(
        org: Option<&str>,
        flag: &str,
        kind: FlagTargetKind,
        key: Option<&str>,
        enabled: bool,
    ) -> FeatureFlagOverride {
        FeatureFlagOverride {
            id: Uuid::new_v4().to_string(),
            org_user_id: org.map(str::to_string),
            flag_key: flag.to_string(),
            target_kind: kind,
            target_key: key.map(str::to_string),
            enabled,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            updated_by: "actor".to_string(),
        }
    }

    #[test]
    fn registry_keys_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for def in FEATURE_FLAGS {
            assert!(seen.insert(def.key), "duplicate flag key {}", def.key);
        }
    }

    #[test]
    fn default_applies_when_no_override() {
        // example_ui defaults to off.
        let enabled = resolve_from_overrides(&[], &[], "user-1", OrgRole::Member);
        assert!(!enabled.contains(&"example_ui".to_string()));
    }

    #[test]
    fn org_override_beats_default() {
        let org = vec![override_row(
            Some("org"),
            "example_ui",
            FlagTargetKind::Org,
            None,
            true,
        )];
        let enabled = resolve_from_overrides(&[], &org, "user-1", OrgRole::Member);
        assert!(enabled.contains(&"example_ui".to_string()));
    }

    #[test]
    fn global_beats_default_and_org_beats_global() {
        let global = vec![override_row(
            None,
            "example_ui",
            FlagTargetKind::Global,
            None,
            true,
        )];
        // Global (on) beats default (off) when there's no org override.
        assert!(
            resolve_from_overrides(&global, &[], "user-1", OrgRole::Member)
                .contains(&"example_ui".to_string())
        );
        // Org (off) wins over global (on) — most specific.
        let org = vec![override_row(
            Some("org"),
            "example_ui",
            FlagTargetKind::Org,
            None,
            false,
        )];
        assert!(
            !resolve_from_overrides(&global, &org, "user-1", OrgRole::Member)
                .contains(&"example_ui".to_string())
        );
    }

    #[test]
    fn role_override_beats_org() {
        let org = vec![
            override_row(Some("org"), "example_ui", FlagTargetKind::Org, None, true),
            override_row(
                Some("org"),
                "example_ui",
                FlagTargetKind::Role,
                Some("viewer"),
                false,
            ),
        ];
        // Viewer: role override (off) wins over org (on).
        let viewer = resolve_from_overrides(&[], &org, "user-1", OrgRole::Viewer);
        assert!(!viewer.contains(&"example_ui".to_string()));
        // Member: no role override, falls back to org (on).
        let member = resolve_from_overrides(&[], &org, "user-1", OrgRole::Member);
        assert!(member.contains(&"example_ui".to_string()));
    }

    #[test]
    fn user_override_beats_role_and_org() {
        let org = vec![
            override_row(Some("org"), "example_ui", FlagTargetKind::Org, None, false),
            override_row(
                Some("org"),
                "example_ui",
                FlagTargetKind::Role,
                Some("member"),
                false,
            ),
            override_row(
                Some("org"),
                "example_ui",
                FlagTargetKind::User,
                Some("user-1"),
                true,
            ),
        ];
        let target = resolve_from_overrides(&[], &org, "user-1", OrgRole::Member);
        assert!(target.contains(&"example_ui".to_string()));
        // A different user in the same role only sees the role/org result (off).
        let other = resolve_from_overrides(&[], &org, "user-2", OrgRole::Member);
        assert!(!other.contains(&"example_ui".to_string()));
    }

    // ── personal (non-org) resolution ─────────────────────────────────────
    #[test]
    fn personal_default_off() {
        assert!(
            !resolve_personal_from_overrides(&[], "user-1").contains(&"example_ui".to_string())
        );
    }

    #[test]
    fn personal_global_then_user_precedence() {
        let platform = vec![
            override_row(None, "example_ui", FlagTargetKind::Global, None, true),
            override_row(
                None,
                "example_ui",
                FlagTargetKind::User,
                Some("user-1"),
                false,
            ),
        ];
        // user-1: personal override (off) wins over global (on).
        assert!(
            !resolve_personal_from_overrides(&platform, "user-1")
                .contains(&"example_ui".to_string())
        );
        // user-2: no personal override, sees global (on).
        assert!(
            resolve_personal_from_overrides(&platform, "user-2")
                .contains(&"example_ui".to_string())
        );
    }

    #[test]
    fn from_parts_validates() {
        assert_eq!(
            FlagTarget::from_parts(FlagTargetKind::Org, None).unwrap(),
            FlagTarget::Org
        );
        assert_eq!(
            FlagTarget::from_parts(FlagTargetKind::Role, Some("admin")).unwrap(),
            FlagTarget::Role(OrgRole::Admin)
        );
        assert!(FlagTarget::from_parts(FlagTargetKind::Role, Some("owner")).is_err());
        assert!(FlagTarget::from_parts(FlagTargetKind::Role, None).is_err());
        assert!(FlagTarget::from_parts(FlagTargetKind::User, None).is_err());
        assert!(FlagTarget::from_parts(FlagTargetKind::User, Some("  ")).is_err());
    }

    #[test]
    fn set_override_rejects_unknown_flag() {
        // Pure validation lives before any DB call, but exercising it needs a
        // db handle; assert the registry lookup instead.
        assert!(find_flag("does-not-exist").is_none());
        assert!(find_flag("example_ui").is_some());
    }

    #[tokio::test]
    async fn set_get_list_and_clear_override() {
        let Some(db) = connect_test_database("feature_flag_crud").await else {
            eprintln!("skipping feature flag service test: no local MongoDB available");
            return;
        };
        let org_id = Uuid::new_v4().to_string();
        let actor = Uuid::new_v4().to_string();

        let stored = set_override(&db, &org_id, "example_ui", &FlagTarget::Org, true, &actor)
            .await
            .expect("set org override");
        assert_eq!(stored.org_user_id.as_deref(), Some(org_id.as_str()));
        assert_eq!(stored.flag_key, "example_ui");
        assert_eq!(stored.target_kind, FlagTargetKind::Org);
        assert!(stored.enabled);
        assert_eq!(stored.updated_by, actor);

        // Idempotent upsert keeps the same row id, flips value.
        let updated = set_override(&db, &org_id, "example_ui", &FlagTarget::Org, false, &actor)
            .await
            .expect("update org override");
        assert_eq!(updated.id, stored.id);
        assert!(!updated.enabled);

        // Resolution reflects the stored override (off).
        let resolved = resolve_enabled_features(&db, &org_id, &actor, OrgRole::Member)
            .await
            .expect("resolve");
        assert!(!resolved.contains(&"example_ui".to_string()));

        // Admin listing shows the flag + its one override.
        let admin = list_for_admin(&db, &org_id).await.expect("admin list");
        let item = admin.iter().find(|i| i.def.key == "example_ui").unwrap();
        assert_eq!(item.overrides.len(), 1);

        // Clear removes it; resolution returns to default (off).
        assert!(
            clear_override(&db, &org_id, "example_ui", &FlagTarget::Org)
                .await
                .expect("clear")
        );
        let after = list_overrides(&db, &org_id)
            .await
            .expect("list after clear");
        assert!(after.is_empty());
    }

    #[tokio::test]
    async fn set_override_rejects_unknown_flag_key_at_db_layer() {
        let Some(db) = connect_test_database("feature_flag_unknown").await else {
            eprintln!("skipping feature flag service test: no local MongoDB available");
            return;
        };
        let err = set_override(
            &db,
            &Uuid::new_v4().to_string(),
            "nope",
            &FlagTarget::Org,
            true,
            "actor",
        )
        .await
        .expect_err("unknown flag rejected");
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn platform_override_resolves_for_non_org_user() {
        let Some(db) = connect_test_database("feature_flag_platform").await else {
            eprintln!("skipping feature flag service test: no local MongoDB available");
            return;
        };
        let user = Uuid::new_v4().to_string();
        let actor = Uuid::new_v4().to_string();

        // A user with zero orgs starts with the code default (off).
        assert!(
            !resolve_personal_features(&db, &user)
                .await
                .expect("resolve personal")
                .contains(&"example_ui".to_string())
        );

        // Global rollout enables it for everyone, org or not.
        let g = set_platform_override(&db, "example_ui", &FlagTarget::Global, true, &actor)
            .await
            .expect("set global");
        assert!(g.org_user_id.is_none());
        assert!(
            resolve_personal_features(&db, &user)
                .await
                .expect("resolve personal")
                .contains(&"example_ui".to_string())
        );

        // A personal per-user override (off) wins over global (on).
        set_platform_override(
            &db,
            "example_ui",
            &FlagTarget::User(user.clone()),
            false,
            &actor,
        )
        .await
        .expect("set personal user");
        assert!(
            !resolve_personal_features(&db, &user)
                .await
                .expect("resolve personal")
                .contains(&"example_ui".to_string())
        );

        // org/role targets are rejected at the platform layer.
        assert!(matches!(
            set_platform_override(&db, "example_ui", &FlagTarget::Org, true, &actor).await,
            Err(AppError::BadRequest(_))
        ));

        // Clearing the global override removes it.
        assert!(
            clear_platform_override(&db, "example_ui", &FlagTarget::Global)
                .await
                .expect("clear global")
        );
    }
}

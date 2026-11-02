//! Admin writes commit flag changes and actor-attributed history together.
use std::collections::HashMap;

use bson::{Bson, Document, doc};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    Database,
    options::{ReturnDocument, WriteConcern},
};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{
    api_key_mutation_service as key_mutations,
    audit_service::{self, AuditActor},
    feature_flag_service::{self as flags, FlagDefinition},
    security_readiness_service,
};
use crate::{
    errors::{AppError, AppResult},
    models::{
        audit_log::{AuditLog, COLLECTION_NAME as AUDIT},
        feature_flag_definition::{COLLECTION_NAME as DEFINITIONS, FeatureFlagDefinition},
        feature_flag_metadata::{COLLECTION_NAME as METADATA, FeatureFlagMetadata},
        feature_flag_override::{
            COLLECTION_NAME as OVERRIDES, FeatureFlagOverride, FlagTargetKind,
        },
        user::{COLLECTION_NAME as USERS, User},
    },
};

pub const MAX_GROUP_LEN: usize = 80;
pub const MAX_TAG_LEN: usize = 40;
pub const MAX_TAGS: usize = 12;

#[derive(Clone, Default)]
pub struct MetadataPatch {
    pub description: Option<Option<String>>,
    pub owner: Option<Option<String>>,
    pub group: Option<Option<String>>,
    pub tags: Option<Vec<String>>,
    pub sunset: Option<bool>,
    pub expected_tags: Option<Vec<String>>,
}

impl MetadataPatch {
    fn update(&self) -> AppResult<Document> {
        let mut set = doc! {};
        for (field, value, limit) in [
            (
                "description",
                &self.description,
                flags::MAX_FLAG_DESCRIPTION_LEN,
            ),
            ("owner", &self.owner, flags::MAX_FLAG_OWNER_LEN),
            ("group", &self.group, MAX_GROUP_LEN),
        ] {
            if let Some(value) = value {
                set.insert(
                    field,
                    flags::normalize_metadata_field(value.as_deref(), limit, field)?
                        .map(Bson::String)
                        .unwrap_or(Bson::Null),
                );
            }
        }
        if let Some(tags) = &self.tags {
            set.insert("tags", normalize_tags(tags)?);
        }
        if let Some(sunset) = self.sunset {
            set.insert("sunset", sunset);
        }
        Ok(set)
    }
}

pub fn normalize_tags(tags: &[String]) -> AppResult<Vec<String>> {
    let mut normalized = Vec::new();
    for tag in tags {
        let tag = tag.trim().to_ascii_lowercase();
        if tag.is_empty()
            || tag.len() > MAX_TAG_LEN
            || !tag.as_bytes()[0].is_ascii_alphanumeric()
            || !tag
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_-".contains(&c))
        {
            return Err(AppError::BadRequest(format!(
                "Tags must be 1-{MAX_TAG_LEN} lowercase letters, digits, hyphens or underscores, starting with a letter or digit"
            )));
        }
        normalized.push(tag);
    }
    normalized.sort();
    normalized.dedup();
    if normalized.len() > MAX_TAGS {
        return Err(AppError::BadRequest(format!(
            "At most {MAX_TAGS} tags are allowed"
        )));
    }
    Ok(normalized)
}

pub enum Mutation {
    Create {
        description: String,
        default_enabled: bool,
    },
    Override {
        kind: FlagTargetKind,
        target_id: Option<String>,
        enabled: Option<bool>,
    },
    Metadata(MetadataPatch),
}

pub struct WriteResult {
    pub definition: FlagDefinition,
    pub metadata: Option<FeatureFlagMetadata>,
    pub override_row: Option<FeatureFlagOverride>,
}

fn metadata_values(row: Option<&FeatureFlagMetadata>) -> Value {
    json!({
        "description": row.and_then(|r| r.description.as_deref()),
        "owner": row.and_then(|r| r.owner.as_deref()),
        "group": row.and_then(|r| r.group.as_deref()),
        "tags": row.map(|r| r.tags.as_slice()).unwrap_or_default(),
        "sunset": row.is_some_and(|r| r.sunset),
    })
}

pub async fn mutate(
    db: &Database,
    key: &str,
    mutation: Mutation,
    actor: &AuditActor,
) -> AppResult<WriteResult> {
    let definition = match &mutation {
        Mutation::Create {
            description,
            default_enabled,
        } => {
            flags::validate_flag_key(key)?;
            if flags::find_flag(key).is_some() {
                return Err(AppError::Conflict("Feature flag already exists".into()));
            }
            let description = flags::normalize_metadata_field(
                Some(description),
                flags::MAX_FLAG_DESCRIPTION_LEN,
                "description",
            )?
            .ok_or_else(|| AppError::BadRequest("description is required".into()))?;
            FlagDefinition {
                created_at: Some(Utc::now()),
                id: Some(Uuid::new_v4().to_string()),
                key: key.into(),
                description,
                default_enabled: *default_enabled,
                created_by: Some(actor.user_id.clone()),
            }
        }
        _ => flags::get_definition(db, key).await?,
    };
    let metadata_set = if let Mutation::Metadata(patch) = &mutation {
        patch.update()?
    } else {
        doc! {}
    };
    if let Mutation::Override {
        kind,
        target_id,
        enabled,
    } = &mutation
    {
        if !matches!(
            kind,
            FlagTargetKind::Global | FlagTargetKind::Org | FlagTargetKind::User
        ) {
            return Err(AppError::BadRequest("Unsupported rollout target".into()));
        }
        if key == flags::INVITATION_CODE_FLAG_KEY && *kind != FlagTargetKind::Global {
            return Err(AppError::BadRequest(
                "invitation code flag supports only a global override".into(),
            ));
        }
        if *kind != FlagTargetKind::Global {
            let id = target_id
                .as_deref()
                .ok_or_else(|| AppError::BadRequest("target_key is required".into()))?;
            if enabled.is_some() {
                if *kind == FlagTargetKind::Org {
                    flags::ensure_org_exists(db, id).await?;
                } else if db
                    .collection::<User>(USERS)
                    .find_one(doc! {"_id":id})
                    .await?
                    .is_none()
                {
                    return Err(AppError::NotFound("User not found".into()));
                }
            }
        }
        // Security-gated capabilities may be disabled at any time but enabled
        // only once every replica reports readiness, for every target scope.
        if *enabled == Some(true) {
            security_readiness_service::require(db, key).await?;
        }
    }
    let db = db.clone();
    let key = key.to_owned();
    let actor = actor.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .write_concern(WriteConcern::majority())
        .and_run2(async move |session| {
            key_mutations::transaction_result(
                mutate_in_session(
                    &db,
                    &key,
                    &definition,
                    &mutation,
                    &metadata_set,
                    &actor,
                    session,
                )
                .await,
            )
        })
        .await
        .map_err(key_mutations::map_transaction_error)
}

async fn mutate_in_session(
    db: &Database,
    key: &str,
    definition: &FlagDefinition,
    mutation: &Mutation,
    metadata_set: &Document,
    actor: &AuditActor,
    session: &mut mongodb::ClientSession,
) -> AppResult<WriteResult> {
    let now = bson::DateTime::now();
    let mut output = WriteResult {
        definition: definition.clone(),
        metadata: None,
        override_row: None,
    };
    let identity = doc! { "flag_key": key, "definition_id": &definition.id };
    let (event_type, details) = match mutation {
        Mutation::Create { .. } => {
            let row = FeatureFlagDefinition {
                id: definition.id.clone().expect("runtime id"),
                flag_key: key.into(),
                description: definition.description.clone(),
                default_enabled: definition.default_enabled,
                created_by: actor.user_id.clone(),
                created_at: definition.created_at.expect("runtime timestamp"),
            };
            if let Err(error) = db
                .collection::<FeatureFlagDefinition>(DEFINITIONS)
                .insert_one(row)
                .session(&mut *session)
                .await
            {
                if matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) if write.code == 11000)
                {
                    return Err(AppError::Conflict("Feature flag already exists".into()));
                }
                return Err(error.into());
            }
            (
                "admin_feature_flag_created",
                json!({"description":definition.description,"default_enabled":definition.default_enabled}),
            )
        }
        Mutation::Metadata(_) => {
            let collection = db.collection::<FeatureFlagMetadata>(METADATA);
            let before = collection
                .find_one(identity.clone())
                .session(&mut *session)
                .await?;
            if let Mutation::Metadata(patch) = mutation
                && let Some(expected) = &patch.expected_tags
                && before
                    .as_ref()
                    .map(|row| row.tags.as_slice())
                    .unwrap_or_default()
                    != expected.as_slice()
            {
                return Err(AppError::Conflict("Flag tags changed. Close this review and use Load latest values in the organization editor before saving again.".into()));
            }
            if metadata_set.is_empty() {
                output.metadata = before;
                return Ok(output);
            }
            let before_values = metadata_values(before.as_ref());
            let mut after_values = before_values.clone();
            for (field, value) in metadata_set {
                after_values[field] = json!(value);
            }
            if before_values == after_values {
                output.metadata = before;
                return Ok(output);
            }
            let mut set = metadata_set.clone();
            set.insert("updated_at", now);
            set.insert("updated_by", &actor.user_id);
            let row = collection.find_one_and_update(identity.clone(), doc! {
                            "$set":set,
                            "$setOnInsert": {"_id":Uuid::new_v4().to_string(),"flag_key":key,"definition_id":&definition.id,"created_at":now}
                        }).upsert(true).return_document(ReturnDocument::After).session(&mut *session).await?
                            .ok_or_else(|| AppError::Internal("Flag metadata update returned no row".into()))?;
            let details = json!({"before":before_values,"after":after_values});
            output.metadata = Some(row);
            ("admin_feature_flag_metadata_set", details)
        }
        Mutation::Override {
            kind,
            target_id,
            enabled,
        } => {
            let mut filter = identity;
            filter.insert("target_kind", kind.as_str());
            filter.insert(
                "org_user_id",
                if *kind == FlagTargetKind::Org {
                    target_id.clone().map(Bson::String).unwrap_or(Bson::Null)
                } else {
                    Bson::Null
                },
            );
            filter.insert(
                "target_key",
                if *kind == FlagTargetKind::User {
                    target_id.clone().map(Bson::String).unwrap_or(Bson::Null)
                } else {
                    Bson::Null
                },
            );
            let collection = db.collection::<FeatureFlagOverride>(OVERRIDES);
            let before = collection
                .find_one(filter.clone())
                .session(&mut *session)
                .await?;
            if before.as_ref().map(|row| row.enabled) == *enabled {
                output.override_row = before;
                return Ok(output);
            }
            output.override_row = if let Some(enabled) = enabled {
                let mut insert = filter.clone();
                insert.insert("_id", Uuid::new_v4().to_string());
                insert.insert("created_at", now);
                collection.find_one_and_update(filter, doc! {"$set":{"enabled":enabled,"updated_at":now,"updated_by":&actor.user_id},"$setOnInsert":insert})
                                .upsert(true).return_document(ReturnDocument::After).session(&mut *session).await?
            } else {
                collection
                    .find_one_and_delete(filter)
                    .session(&mut *session)
                    .await?
            };
            (
                if enabled.is_some() {
                    "admin_feature_flag_override_set"
                } else {
                    "admin_feature_flag_override_cleared"
                },
                json!({"target_kind":kind.as_str(),"target_key":target_id,"org_user_id":if *kind == FlagTargetKind::Org {target_id.as_deref()} else {None},"enabled":enabled,"before":before.map(|r|r.enabled),"after":enabled}),
            )
        }
    };
    let mut details = details.as_object().expect("event object").clone();
    details.insert("flag_key".into(), json!(key));
    details.insert("definition_id".into(), json!(definition.id));
    audit_service::log_actor_event_in_transaction(
        db,
        session,
        actor,
        event_type,
        Value::Object(details),
    )
    .await?;
    Ok(output)
}

#[derive(serde::Deserialize)]
pub struct HistoryActor {
    #[serde(rename = "_id")]
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub slug: Option<String>,
}

pub struct HistoryPage {
    pub entries: Vec<AuditLog>,
    pub actors: HashMap<String, HistoryActor>,
    pub next_cursor: Option<String>,
    pub legacy_ambiguous_omitted: bool,
}

pub async fn history(
    db: &Database,
    def: &FlagDefinition,
    cursor: Option<&str>,
) -> AppResult<HistoryPage> {
    let mut identity_clauses = vec![match &def.id {
        Some(id) => doc! {"event_data.definition_id": id},
        None => doc! {"event_data.definition_id": {"$type":"null"}},
    }];
    let mut legacy_ambiguous_omitted = false;
    if let Some(created_at) = def.created_at {
        identity_clauses.push(doc! {"event_data.definition_id":{"$exists":false},"created_at":{"$gte":bson::DateTime::from_chrono(created_at)}});
    } else {
        let shadow = db
            .collection::<FeatureFlagDefinition>(DEFINITIONS)
            .find_one(doc! {"flag_key":&def.key})
            .await?;
        let mut legacy = doc! {"event_data.definition_id":{"$exists":false}};
        if let Some(shadow) = shadow {
            legacy.insert(
                "created_at",
                doc! {"$lt":bson::DateTime::from_chrono(shadow.created_at)},
            );
            legacy_ambiguous_omitted = true;
        }
        identity_clauses.push(legacy);
    }
    let mut clauses = vec![
        doc! {"event_data.flag_key":&def.key,"event_type":{"$in":["admin_feature_flag_created","admin_feature_flag_override_set","admin_feature_flag_override_cleared","admin_feature_flag_metadata_set"]}},
        doc! {"$or":identity_clauses},
    ];
    if let Some(cursor) = cursor {
        let (timestamp, id) = cursor
            .split_once(':')
            .ok_or_else(|| AppError::BadRequest("Invalid history cursor".into()))?;
        let millis = timestamp
            .parse::<i64>()
            .map_err(|_| AppError::BadRequest("Invalid history cursor".into()))?;
        Uuid::parse_str(id).map_err(|_| AppError::BadRequest("Invalid history cursor".into()))?;
        let at = bson::DateTime::from_millis(millis);
        clauses.push(doc! {"$or":[{"created_at":{"$lt":at}},{"created_at":at,"_id":{"$lt":id}}]});
    }
    let mut entries: Vec<AuditLog> = db
        .collection::<AuditLog>(AUDIT)
        .find(doc! {"$and":clauses})
        .sort(doc! {"created_at":-1,"_id":-1})
        .limit(26)
        .await?
        .try_collect()
        .await?;
    let has_more = entries.len() > 25;
    entries.truncate(25);
    let next_cursor = if has_more {
        entries
            .last()
            .map(|entry| format!("{}:{}", entry.created_at.timestamp_millis(), entry.id))
    } else {
        None
    };
    let mut actor_ids: Vec<_> = entries.iter().filter_map(|e| e.user_id.clone()).collect();
    actor_ids.extend(def.created_by.iter().cloned());
    actor_ids.extend(entries.iter().filter_map(|entry| {
        let data = entry.event_data.as_ref()?;
        data.get("org_user_id")
            .and_then(Value::as_str)
            .or_else(|| data.get("target_key").and_then(Value::as_str))
            .map(str::to_owned)
    }));
    let actors = db
        .collection::<HistoryActor>(USERS)
        .find(doc! {"_id":{"$in":actor_ids}})
        .projection(doc! {"_id":1,"email":1,"display_name":1,"slug":1})
        .await?
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .map(|user| (user.id.clone(), user))
        .collect();
    Ok(HistoryPage {
        entries,
        actors,
        next_cursor,
        legacy_ambiguous_omitted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::UserType;
    use crate::test_utils::{
        connect_transaction_test_database, init_test_audit_chain_hmac_key, test_user,
    };

    fn actor() -> AuditActor {
        AuditActor {
            user_id: Uuid::new_v4().to_string(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        }
    }
    async fn setup(name: &str) -> (Database, AuditActor) {
        init_test_audit_chain_hmac_key();
        let db = connect_transaction_test_database(name).await;
        let actor = actor();
        db.collection::<User>(USERS)
            .insert_one(test_user(&actor.user_id, UserType::Person))
            .await
            .unwrap();
        db.collection::<Document>(DEFINITIONS)
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(doc! {"flag_key":1})
                    .options(
                        mongodb::options::IndexOptions::builder()
                            .unique(true)
                            .build(),
                    )
                    .build(),
            )
            .await
            .unwrap();
        (db, actor)
    }
    async fn create(db: &Database, actor: &AuditActor) -> FlagDefinition {
        mutate(
            db,
            "team:history",
            Mutation::Create {
                description: "History test".into(),
                default_enabled: false,
            },
            actor,
        )
        .await
        .unwrap()
        .definition
    }
    #[test]
    fn tag_validation_normalizes_and_bounds() {
        assert_eq!(
            normalize_tags(&[" Voice ".into(), "beta".into(), "voice".into()]).unwrap(),
            vec!["beta", "voice"]
        );
        for tags in [
            vec!["bad tag".into()],
            vec!["".into()],
            vec!["x".repeat(41)],
            (0..13).map(|i| format!("tag{i}")).collect(),
        ] {
            assert!(normalize_tags(&tags).is_err());
        }
        assert_eq!(normalize_tags(&vec!["a".into(); 13]).unwrap(), vec!["a"]);
    }
    #[tokio::test]
    async fn sunset_is_metadata_only_and_sparse_writes_preserve_it() {
        let (db, actor) = setup("flag_sunset").await;
        let def = create(&db, &actor).await;
        mutate(
            &db,
            &def.key,
            Mutation::Override {
                kind: FlagTargetKind::Global,
                target_id: None,
                enabled: Some(true),
            },
            &actor,
        )
        .await
        .unwrap();
        mutate(
            &db,
            &def.key,
            Mutation::Metadata(MetadataPatch {
                sunset: Some(true),
                group: Some(Some("Assistant".into())),
                tags: Some(vec!["voice".into()]),
                ..Default::default()
            }),
            &actor,
        )
        .await
        .unwrap();
        mutate(
            &db,
            &def.key,
            Mutation::Metadata(MetadataPatch {
                owner: Some(Some("Team".into())),
                ..Default::default()
            }),
            &actor,
        )
        .await
        .unwrap();
        let saved = flags::list_metadata(&db)
            .await
            .unwrap()
            .remove(&def.key)
            .unwrap();
        assert!(saved.sunset);
        assert_eq!(saved.group.as_deref(), Some("Assistant"));
        assert_eq!(saved.tags, vec!["voice"]);
        assert!(
            flags::resolve_personal_features(&db, &actor.user_id)
                .await
                .unwrap()
                .contains(&def.key)
        );
        assert!(
            flags::flag_enabled_people(&db, &def.key)
                .await
                .unwrap()
                .is_none()
        );
        let history = history(&db, &def, None).await.unwrap();
        assert_eq!(history.entries.len(), 4);
        assert_eq!(
            history.entries[0].event_data.as_ref().unwrap()["after"]["sunset"],
            true
        );
        assert!(
            history
                .entries
                .iter()
                .all(|row| row.user_id.as_deref() == Some(&actor.user_id)
                    && row.seq.is_some()
                    && row.prev_hash.is_some()
                    && row.entry_hash.is_some())
        );
        let restored = mutate(
            &db,
            &def.key,
            Mutation::Metadata(MetadataPatch {
                sunset: Some(false),
                ..Default::default()
            }),
            &actor,
        )
        .await
        .unwrap();
        assert!(!restored.metadata.unwrap().sunset);
    }
    #[tokio::test]
    async fn creation_and_change_roll_back_if_history_cannot_be_written() {
        let (db, actor) = setup("flag_audit_atomic").await;
        db.create_collection(AUDIT).await.unwrap();
        db.run_command(doc! {"collMod":AUDIT,"validator":{"event_type":{"$eq":"rejected"}},"validationLevel":"strict"}).await.unwrap();
        assert!(
            mutate(
                &db,
                "team:history",
                Mutation::Create {
                    description: "Test".into(),
                    default_enabled: false
                },
                &actor
            )
            .await
            .is_err()
        );
        assert!(
            db.collection::<Document>(DEFINITIONS)
                .find_one(doc! {"flag_key":"team:history"})
                .await
                .unwrap()
                .is_none()
        );
        db.run_command(doc! {"collMod":AUDIT,"validator":{}})
            .await
            .unwrap();
        let def = create(&db, &actor).await;
        let saved = mutate(
            &db,
            &def.key,
            Mutation::Metadata(MetadataPatch {
                group: Some(Some("Before".into())),
                ..Default::default()
            }),
            &actor,
        )
        .await
        .unwrap()
        .metadata
        .unwrap();
        db.run_command(doc! {"collMod":AUDIT,"validator":{"event_type":{"$eq":"rejected"}}})
            .await
            .unwrap();
        for mutation in [
            Mutation::Metadata(MetadataPatch {
                sunset: Some(true),
                ..Default::default()
            }),
            Mutation::Override {
                kind: FlagTargetKind::Global,
                target_id: None,
                enabled: Some(true),
            },
        ] {
            assert!(mutate(&db, &def.key, mutation, &actor).await.is_err());
        }
        let unchanged = db
            .collection::<FeatureFlagMetadata>(METADATA)
            .find_one(doc! {"_id":&saved.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            bson::to_document(&saved).unwrap(),
            bson::to_document(&unchanged).unwrap()
        );
        assert_eq!(
            db.collection::<Document>(OVERRIDES)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(history(&db, &def, None).await.unwrap().entries.len(), 2);
        db.run_command(doc! {"collMod":AUDIT,"validator":{}})
            .await
            .unwrap();
        for mutation in [
            Mutation::Metadata(MetadataPatch {
                sunset: Some(true),
                ..Default::default()
            }),
            Mutation::Override {
                kind: FlagTargetKind::Global,
                target_id: None,
                enabled: Some(true),
            },
        ] {
            mutate(&db, &def.key, mutation, &actor).await.unwrap();
        }
        let report = super::super::audit_chain_service::verify_chain(
            &db,
            audit_service::audit_chain_hmac_key().unwrap(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            report.status,
            super::super::audit_chain_service::AuditChainStatus::Ok
        );
        assert_eq!(report.checked_count, 4);
        assert_eq!(report.pre_chain_count, 0);
    }
    #[tokio::test]
    async fn security_readiness_gates_enabling_capabilities_for_every_scope() {
        let (db, actor) = setup("flag_admin_readiness").await;
        let org = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&org, UserType::Org))
            .await
            .unwrap();
        let flag = flags::FEATURE_FLAGS
            .iter()
            .find(|flag| security_readiness_service::CAPABILITIES.contains(&flag.key))
            .expect("a security-gated capability flag");
        let targets = [
            (FlagTargetKind::Global, None),
            (FlagTargetKind::User, Some(actor.user_id.clone())),
            (FlagTargetKind::Org, Some(org.clone())),
        ];
        let set = |kind: FlagTargetKind, target_id: Option<String>, enabled: bool| {
            mutate(
                &db,
                flag.key,
                Mutation::Override {
                    kind,
                    target_id,
                    enabled: Some(enabled),
                },
                &actor,
            )
        };
        for (kind, target_id) in targets.clone() {
            assert!(matches!(
                set(kind, target_id.clone(), true).await,
                Err(AppError::SecurityReadinessUnavailable)
            ));
            // Disabling is always allowed.
            let row = set(kind, target_id, false).await.unwrap().override_row;
            assert!(!row.expect("disabled override").enabled);
        }
        security_readiness_service::ready_test_replica(&db).await;
        for (kind, target_id) in targets {
            let row = set(kind, target_id, true).await.unwrap().override_row;
            assert!(row.expect("enabled override").enabled);
        }
    }
    #[tokio::test]
    async fn tags_detect_unseen_conflicts_and_noop_writes_do_not_add_history() {
        let (db, actor) = setup("flag_tag_conflict").await;
        let def = create(&db, &actor).await;
        let unchanged = mutate(
            &db,
            &def.key,
            Mutation::Metadata(MetadataPatch {
                tags: Some(vec![]),
                ..Default::default()
            }),
            &actor,
        )
        .await
        .unwrap();
        assert!(unchanged.metadata.is_none());
        assert_eq!(
            db.collection::<Document>(METADATA)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        let patch = || {
            Mutation::Metadata(MetadataPatch {
                tags: Some(vec!["voice".into()]),
                ..Default::default()
            })
        };
        let first = mutate(&db, &def.key, patch(), &actor)
            .await
            .unwrap()
            .metadata
            .unwrap();
        let other_actor = AuditActor {
            user_id: Uuid::new_v4().to_string(),
            ..actor.clone()
        };
        let repeated = mutate(&db, &def.key, patch(), &other_actor)
            .await
            .unwrap()
            .metadata
            .unwrap();
        assert_eq!(
            bson::to_document(&first).unwrap(),
            bson::to_document(&repeated).unwrap()
        );
        assert_eq!(history(&db, &def, None).await.unwrap().entries.len(), 2);
        let err = mutate(
            &db,
            &def.key,
            Mutation::Metadata(MetadataPatch {
                tags: Some(vec!["beta".into()]),
                expected_tags: Some(vec![]),
                ..Default::default()
            }),
            &actor,
        )
        .await
        .err()
        .unwrap();
        assert!(matches!(err, AppError::Conflict(_)));
    }
    #[tokio::test]
    async fn repeated_rollout_does_not_change_attribution_or_history() {
        let (db, actor) = setup("flag_rollout_noop").await;
        let def = create(&db, &actor).await;
        let change = || Mutation::Override {
            kind: FlagTargetKind::Global,
            target_id: None,
            enabled: Some(true),
        };
        let first = mutate(&db, &def.key, change(), &actor)
            .await
            .unwrap()
            .override_row
            .unwrap();
        let other = AuditActor {
            user_id: Uuid::new_v4().to_string(),
            ..actor.clone()
        };
        let repeated = mutate(&db, &def.key, change(), &other)
            .await
            .unwrap()
            .override_row
            .unwrap();
        assert_eq!(
            bson::to_document(&first).unwrap(),
            bson::to_document(&repeated).unwrap()
        );
        assert_eq!(history(&db, &def, None).await.unwrap().entries.len(), 2);
    }
    #[tokio::test]
    async fn cursor_history_has_no_gaps_and_records_before_after() {
        let (db, actor) = setup("flag_cursor").await;
        let def = create(&db, &actor).await;
        for i in 0..27 {
            mutate(
                &db,
                &def.key,
                Mutation::Override {
                    kind: FlagTargetKind::Global,
                    target_id: None,
                    enabled: Some(i % 2 == 0),
                },
                &actor,
            )
            .await
            .unwrap();
        }
        let first = history(&db, &def, None).await.unwrap();
        assert_eq!(first.entries.len(), 25);
        let second = history(&db, &def, first.next_cursor.as_deref())
            .await
            .unwrap();
        assert_eq!(second.entries.len(), 3);
        assert!(second.next_cursor.is_none());
        assert!(
            !second
                .entries
                .iter()
                .any(|row| first.entries.iter().any(|other| other.id == row.id))
        );
        assert_eq!(
            first.entries[0].event_data.as_ref().unwrap()["before"],
            false
        );
        assert_eq!(first.entries[0].event_data.as_ref().unwrap()["after"], true);
    }
    #[tokio::test]
    async fn legacy_history_respects_runtime_and_builtin_identity() {
        let (db, actor) = setup("flag_legacy_identity").await;
        let def = create(&db, &actor).await;
        audit_service::log_actor_event(
            db.clone(),
            &actor,
            "admin_feature_flag_override_set",
            Some(json!({"flag_key":def.key,"enabled":true,"target_kind":"global"})),
        )
        .await
        .unwrap();
        let page = history(&db, &def, None).await.unwrap();
        assert_eq!(page.entries.len(), 2);
        // Built-in and runtime settings for a reused key must never mix.
        db.collection::<FeatureFlagDefinition>(DEFINITIONS)
            .insert_one(FeatureFlagDefinition {
                id: Uuid::new_v4().to_string(),
                flag_key: "example_ui".into(),
                description: "Shadowed runtime".into(),
                default_enabled: false,
                created_by: actor.user_id.clone(),
                created_at: Utc::now() - chrono::Duration::days(1),
            })
            .await
            .unwrap();
        audit_service::log_actor_event(
            db.clone(),
            &actor,
            "admin_feature_flag_override_set",
            Some(json!({"flag_key":"example_ui","enabled":true})),
        )
        .await
        .unwrap();
        audit_service::log_actor_event(
            db.clone(),
            &actor,
            "admin_feature_flag_override_set",
            Some(json!({"flag_key":"example_ui","definition_id":null,"enabled":false})),
        )
        .await
        .unwrap();
        let builtin = flags::get_definition(&db, "example_ui").await.unwrap();
        let page = history(&db, &builtin, None).await.unwrap();
        assert!(page.legacy_ambiguous_omitted);
        assert_eq!(page.entries.len(), 1);
        assert_eq!(
            page.entries[0].event_data.as_ref().unwrap()["enabled"],
            false
        );
    }
}

use std::collections::{BTreeMap, HashMap, HashSet};

use futures::TryStreamExt;
use mongodb::{Database, bson::doc, options::ReturnDocument};
use uuid::Uuid;

use crate::{
    errors::{AppError, AppResult},
    models::service_preference::{COLLECTION_NAME, ServicePreference},
    services::{platform_key_service, unified_key_service::KeyView},
};

pub const MAX_ORDERED_SERVICES: usize = 200;
pub const MAX_EXPECTED_VERSION: i64 = 9_007_199_254_740_990;
pub const MAX_REQUEST_BYTES: usize = 16 * 1024;

pub async fn get(db: &Database, user_id: &str) -> AppResult<Option<ServicePreference>> {
    Ok(db
        .collection::<ServicePreference>(COLLECTION_NAME)
        .find_one(doc! { "_id": user_id })
        .await?)
}

pub fn group_key(service_id: &str, catalog_service_id: Option<&str>) -> String {
    catalog_service_id.map_or_else(
        || format!("connection:{service_id}"),
        |id| format!("catalog:{id}"),
    )
}

pub struct PreferenceMember {
    pub group: String,
    pub listed: bool,
}

pub fn rank_map_by_group(
    ordered: &[String],
    visible: &HashMap<String, PreferenceMember>,
) -> HashMap<String, u32> {
    let mut counts = HashMap::<&str, u32>::new();
    let mut ranks = HashMap::new();
    for id in ordered {
        if let Some(member) = visible.get(id)
            && member.listed
            && member.group.starts_with("catalog:")
            && !ranks.contains_key(id)
        {
            let rank = counts.entry(&member.group).or_default();
            *rank += 1;
            ranks.insert(id.clone(), *rank);
        }
    }
    ranks
}

pub fn position_map_by_group(
    ordered: &[String],
    visible: &HashMap<String, PreferenceMember>,
) -> HashMap<String, u32> {
    let mut positions = HashMap::new();
    let mut counts = HashMap::<&str, u32>::new();
    for id in ordered {
        if let Some(member) = visible.get(id)
            && member.group.starts_with("catalog:")
            && !positions.contains_key(id)
        {
            let position = counts.entry(&member.group).or_default();
            *position += 1;
            positions.insert(id.clone(), *position);
        }
    }
    positions
}

pub fn grouped_visible(
    ordered: &[String],
    visible: &HashMap<String, PreferenceMember>,
) -> BTreeMap<String, Vec<String>> {
    let mut groups = BTreeMap::<String, Vec<String>>::new();
    let mut seen = HashSet::new();
    for id in ordered {
        if let Some(member) = visible.get(id)
            && member.group.starts_with("catalog:")
            && seen.insert(id)
        {
            groups
                .entry(member.group.clone())
                .or_default()
                .push(id.clone());
        }
    }
    groups
}

pub fn filter_inventory(
    views: Vec<KeyView>,
    scope: Option<&[String]>,
    api_key: bool,
) -> Vec<KeyView> {
    views
        .into_iter()
        .filter(|view| scope.is_none_or(|ids| ids.contains(&view.id)))
        .filter(|view| !api_key || !view.credential_source.is_viewer_org())
        .collect()
}

pub fn validate_group(group: &str) -> AppResult<&str> {
    group
        .strip_prefix("catalog:")
        .filter(|id| {
            Uuid::parse_str(id).is_ok_and(|uuid| {
                uuid.get_variant() == uuid::Variant::RFC4122
                    && (1..=8).contains(&uuid.get_version_num())
                    && uuid.to_string() == *id
            })
        })
        .ok_or_else(|| {
            AppError::ValidationError("agent order requires a canonical catalog group".into())
        })
}

fn canonical_uuid(id: &str) -> bool {
    Uuid::parse_str(id).is_ok_and(|uuid| {
        uuid.get_version_num() == 4
            && uuid.get_variant() == uuid::Variant::RFC4122
            && uuid.to_string() == id
    })
}

pub fn validate_order(
    ordered: &[String],
    expected_version: i64,
    visible: &HashSet<String>,
) -> AppResult<()> {
    if !(0..=MAX_EXPECTED_VERSION).contains(&expected_version) {
        return Err(AppError::ValidationError(
            "invalid preference version".into(),
        ));
    }
    if ordered.len() > MAX_ORDERED_SERVICES {
        return Err(AppError::ValidationError("Agent order storage is full (200 connections across all services). Reset the agent order of another service, or release unavailable preferences for services you can no longer access, then try again.".into()));
    }
    let mut seen = HashSet::new();
    for id in ordered {
        if !canonical_uuid(id) {
            return Err(AppError::ValidationError(
                "service ids must be canonical UUID v4 strings".into(),
            ));
        }
        if !seen.insert(id) {
            return Err(AppError::ValidationError("duplicate service id".into()));
        }
        if !visible.contains(id) {
            return Err(AppError::ValidationError("unknown service id".into()));
        }
    }
    Ok(())
}

/// Replace only occupied slots belonging to visible members of the target group.
/// Extra submitted IDs follow its last slot; untouched IDs retain their order.
pub fn merge_group(stored: &[String], ordered: &[String], target: &HashSet<String>) -> Vec<String> {
    let mut effective = ordered.to_vec();
    if !ordered.is_empty() {
        for id in stored {
            if target.contains(id) && !effective.contains(id) {
                effective.push(id.clone());
            }
        }
    }
    merge_slots(stored, &effective, target)
}

fn merge_slots(stored: &[String], ordered: &[String], target: &HashSet<String>) -> Vec<String> {
    let logical: Vec<_> = stored
        .iter()
        .filter(|id| target.contains(*id))
        .cloned()
        .collect();
    if logical == ordered {
        return stored.to_vec();
    }
    let last_slot = stored.iter().rposition(|id| target.contains(id));
    let mut remaining = ordered.iter();
    let mut merged = Vec::new();
    for (index, id) in stored.iter().enumerate() {
        if target.contains(id) {
            if let Some(next) = remaining.next() {
                merged.push(next.clone());
            }
        } else {
            merged.push(id.clone());
        }
        if Some(index) == last_slot {
            merged.extend(remaining.by_ref().cloned());
        }
    }
    merged.extend(remaining.cloned());
    merged
}

pub struct Replacement {
    pub preference: Option<ServicePreference>,
    pub changed: bool,
}

pub async fn replace_group(
    db: &Database,
    user_id: &str,
    group: &str,
    ordered: &[String],
    expected_version: i64,
) -> AppResult<Replacement> {
    let catalog = validate_group(group)?;
    // Shape validation precedes any inventory query.
    validate_order(
        ordered,
        expected_version,
        &ordered.iter().cloned().collect(),
    )?;
    let current = get(db, user_id).await?;
    if current.as_ref().map_or(0, |row| row.version) != expected_version {
        return Err(AppError::Conflict(
            "preference order changed elsewhere".into(),
        ));
    }
    let stored = current
        .as_ref()
        .map_or(&[][..], |row| row.ordered.as_slice());
    let selected: Vec<_> = stored
        .iter()
        .chain(ordered)
        .cloned()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let members = visible_members(db, user_id, Some(&selected), Some(catalog), None, false).await?;
    let target = members.keys().cloned().collect();
    validate_order(ordered, expected_version, &target)?;
    let merged = merge_group(stored, ordered, &target);
    if merged == stored {
        return Ok(Replacement {
            preference: current,
            changed: false,
        });
    }
    if merged.len() > MAX_ORDERED_SERVICES {
        return Err(AppError::ValidationError("Agent order storage is full (200 connections across all services). Reset the agent order of another service, or release unavailable preferences for services you can no longer access, then try again.".into()));
    }
    persist(db, user_id, current, merged, expected_version).await
}

pub struct ReleasedHidden {
    pub replacement: Replacement,
    pub released_hidden: usize,
}

pub async fn release_hidden(
    db: &Database,
    user_id: &str,
    expected_version: i64,
) -> AppResult<ReleasedHidden> {
    validate_order(&[], expected_version, &HashSet::new())?;
    let current = get(db, user_id).await?;
    if current.as_ref().map_or(0, |row| row.version) != expected_version {
        return Err(AppError::Conflict(
            "preference order changed elsewhere".into(),
        ));
    }
    let stored = current
        .as_ref()
        .map_or(&[][..], |row| row.ordered.as_slice());
    let visible = visible_ordered_members(db, user_id, stored, None, false).await?;
    let merged: Vec<_> = stored
        .iter()
        .filter(|id| visible.contains_key(*id))
        .cloned()
        .collect();
    let released_hidden = stored.len() - merged.len();
    let replacement = if released_hidden == 0 {
        Replacement {
            preference: current,
            changed: false,
        }
    } else {
        persist(db, user_id, current, merged, expected_version).await?
    };
    Ok(ReleasedHidden {
        replacement,
        released_hidden,
    })
}

async fn persist(
    db: &Database,
    user_id: &str,
    current: Option<ServicePreference>,
    merged: Vec<String>,
    expected_version: i64,
) -> AppResult<Replacement> {
    let collection = db.collection::<ServicePreference>(COLLECTION_NAME);
    let now = bson::DateTime::now().to_chrono();
    let preference = if current.is_none() {
        let row = ServicePreference {
            user_id: user_id.into(),
            ordered: merged,
            version: 1,
            created_at: now,
            updated_at: now,
        };
        match collection.insert_one(&row).await {
            Ok(_) => row,
            Err(error) if duplicate_key(&error) => {
                return Err(AppError::Conflict(
                    "preference order changed elsewhere".into(),
                ));
            }
            Err(error) => return Err(error.into()),
        }
    } else {
        let filter = if expected_version == 0 {
            doc! { "_id": user_id, "$or": [{ "version": 0_i64 }, { "version": { "$exists": false } }] }
        } else {
            doc! { "_id": user_id, "version": expected_version }
        };
        collection.find_one_and_update(filter,
            doc! { "$set": { "ordered": &merged, "updated_at": bson::DateTime::from_chrono(now) }, "$inc": { "version": 1_i64 } })
            .return_document(ReturnDocument::After).await?
            .ok_or_else(|| AppError::Conflict("preference order changed elsewhere".into()))?
    };
    Ok(Replacement {
        preference: Some(preference),
        changed: true,
    })
}

fn duplicate_key(error: &mongodb::error::Error) -> bool {
    use mongodb::error::{ErrorKind, WriteFailure};
    matches!(error.kind.as_ref(), ErrorKind::Write(WriteFailure::WriteError(error)) if error.code == 11000)
        || matches!(error.kind.as_ref(), ErrorKind::Command(error) if error.code == 11000)
}

/// Load ranks without changing the original discovery vector.
pub async fn order_discovery(
    db: &Database,
    user_id: &str,
    services: Vec<super::mcp_service::McpToolService>,
) -> AppResult<(
    Vec<super::mcp_service::McpToolService>,
    HashMap<String, u32>,
)> {
    let visible = services
        .iter()
        .filter_map(|service| match &service.source {
            super::mcp_service::McpToolSource::UserManaged {
                catalog_service_id, ..
            } => Some((
                service.service_id.clone(),
                PreferenceMember {
                    group: group_key(&service.service_id, catalog_service_id.as_deref()),
                    listed: true,
                },
            )),
            _ => None,
        })
        .collect();
    let preference = get(db, user_id).await?;
    let ranks = rank_map_by_group(
        preference
            .as_ref()
            .map_or(&[], |row| row.ordered.as_slice()),
        &visible,
    );
    Ok((services, ranks))
}

pub async fn detail_rank(
    db: &Database,
    user_id: &str,
    service_id: &str,
    catalog_service_id: Option<&str>,
    scope: Option<&[String]>,
    api_key: bool,
) -> AppResult<(Option<u32>, Option<u32>)> {
    let Some(catalog) = catalog_service_id else {
        return Ok((None, None));
    };
    let Some(preference) = get(db, user_id).await? else {
        return Ok((None, None));
    };
    if !preference.ordered.iter().any(|id| id == service_id) {
        return Ok((None, None));
    }
    let selected: Vec<_> = preference
        .ordered
        .iter()
        .take(MAX_ORDERED_SERVICES)
        .cloned()
        .collect();
    let visible =
        visible_members(db, user_id, Some(&selected), Some(catalog), scope, api_key).await?;
    Ok((
        rank_map_by_group(&preference.ordered, &visible)
            .get(service_id)
            .copied(),
        position_map_by_group(&preference.ordered, &visible)
            .get(service_id)
            .copied(),
    ))
}

pub async fn visible_ordered_members(
    db: &Database,
    user_id: &str,
    ordered: &[String],
    scope: Option<&[String]>,
    api_key: bool,
) -> AppResult<HashMap<String, PreferenceMember>> {
    let selected: Vec<_> = ordered.iter().take(MAX_ORDERED_SERVICES).cloned().collect();
    visible_members(db, user_id, Some(&selected), None, scope, api_key).await
}

/// Reuse live source/membership visibility, selecting IDs and an optional immutable
/// catalog group without rendering or decrypting key/provider metadata.
async fn visible_members(
    db: &Database,
    user_id: &str,
    selected: Option<&[String]>,
    catalog: Option<&str>,
    scope: Option<&[String]>,
    api_key: bool,
) -> AppResult<HashMap<String, PreferenceMember>> {
    let selected: Option<Vec<_>> = selected.map(|ids| {
        ids.iter()
            .filter(|id| scope.is_none_or(|scope| scope.contains(id)))
            .cloned()
            .collect()
    });
    if selected.as_ref().is_some_and(Vec::is_empty) {
        return Ok(HashMap::new());
    }
    let grants = platform_key_service::OwnerGrants::load_for_listing(db, user_id).await?;
    let tagged = super::user_service_service::list_user_services_with_sources_selected_in_group(
        db,
        user_id,
        false,
        true,
        grants.memberships(),
        super::user_service_service::ServiceSelection {
            ids: selected.as_deref(),
            catalog_service_id: catalog,
        },
    )
    .await?;
    let tagged: Vec<_> = tagged
        .into_iter()
        .filter(|row| !api_key || !row.source.is_viewer_org())
        .collect();
    if tagged.is_empty() {
        return Ok(HashMap::new());
    }
    let endpoint_ids: Vec<_> = tagged.iter().map(|row| &row.service.endpoint_id).collect();
    let endpoints: Vec<bson::Document> = super::service_history::collection::<bson::Document>(
        db,
        crate::models::user_endpoint::COLLECTION_NAME,
    )
    .find(doc! {"_id": {"$in": endpoint_ids}})
    .projection(doc! {"_id":1})
    .await?
    .try_collect()
    .await?;
    let endpoints: HashSet<_> = endpoints
        .iter()
        .filter_map(|row| row.get_str("_id").ok())
        .collect();
    Ok(tagged
        .iter()
        .filter(|row| endpoints.contains(row.service.endpoint_id.as_str()))
        .map(|row| {
            (
                row.service.id.clone(),
                PreferenceMember {
                    group: group_key(&row.service.id, row.service.catalog_service_id.as_deref()),
                    listed: row.service.is_active && row.service.service_type == "http",
                },
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn service_preference_validation_and_dense_visibility() {
        let a = Uuid::new_v4().to_string();
        let b = Uuid::new_v4().to_string();
        let visible = HashSet::from([a.clone()]);
        for (ids, version) in [
            (vec![a.clone(); 201], 0),
            (vec!["invalid".into()], 0),
            (vec![a.clone(), a.clone()], 0),
            (vec![a.to_uppercase()], 0),
            (vec![Uuid::nil().to_string()], 0),
            (vec![], -1),
            (vec![], MAX_EXPECTED_VERSION + 1),
        ] {
            assert!(validate_order(&ids, version, &visible).is_err());
        }
        assert!(validate_group(&format!("connection:{a}")).is_err());
        assert!(validate_group("catalog:invalid").is_err());
        assert!(validate_group(&format!("catalog:{a}")).is_ok());
        for id in [
            "00000000-0000-0000-0000-000000000000",
            "ffffffff-ffff-ffff-ffff-ffffffffffff",
            "aaaaaaaa-aaaa-9aaa-8aaa-aaaaaaaaaaaa",
            "aaaaaaaa-aaaa-4aaa-1aaa-aaaaaaaaaaaa",
            "AAAAAAAA-AAAA-4AAA-8AAA-AAAAAAAAAAAA",
        ] {
            assert!(validate_group(&format!("catalog:{id}")).is_err(), "{id}");
        }
        for version in 1..=8 {
            assert!(
                validate_group(&format!(
                    "catalog:aaaaaaaa-aaaa-{version}aaa-8aaa-aaaaaaaaaaaa"
                ))
                .is_ok()
            );
        }
        let bad_variant = "aaaaaaaa-aaaa-4aaa-1aaa-aaaaaaaaaaaa".to_string();
        assert!(
            validate_order(
                std::slice::from_ref(&bad_variant),
                0,
                &HashSet::from([bad_variant.clone()])
            )
            .is_err()
        );
        let c = Uuid::new_v4().to_string();
        let d = Uuid::new_v4().to_string();
        let members = HashMap::from([
            (
                a.clone(),
                PreferenceMember {
                    group: "catalog:a".into(),
                    listed: true,
                },
            ),
            (
                b.clone(),
                PreferenceMember {
                    group: "catalog:a".into(),
                    listed: false,
                },
            ),
            (
                c.clone(),
                PreferenceMember {
                    group: "catalog:b".into(),
                    listed: true,
                },
            ),
            (
                d.clone(),
                PreferenceMember {
                    group: format!("connection:{d}"),
                    listed: true,
                },
            ),
        ]);
        assert_eq!(
            rank_map_by_group(&[b, c.clone(), d, a.clone()], &members),
            HashMap::from([(a, 1), (c, 1)])
        );
    }
    #[test]
    fn service_preference_merge_preserves_slots_hidden_and_logical_noop() {
        let stored = vec![
            "a".into(),
            "hidden".into(),
            "other".into(),
            "b".into(),
            "stale".into(),
        ];
        let target = HashSet::from(["a".into(), "b".into(), "new".into()]);
        assert_eq!(
            merge_group(&stored, &["a".into(), "b".into()], &target),
            stored
        );
        assert_eq!(
            merge_group(&stored, &["b".into(), "a".into(), "new".into()], &target),
            vec!["b", "hidden", "other", "a", "new", "stale"]
        );
        assert_eq!(
            merge_group(&stored, &[], &target),
            vec!["hidden", "other", "stale"]
        );
    }
    #[test]
    fn service_preference_merge_varied_inventory_sequences_preserve_hidden_and_regained_scope() {
        fn shuffle(items: &mut [String], seed: &mut u64) {
            for i in (1..items.len()).rev() {
                *seed ^= *seed << 13;
                *seed ^= *seed >> 7;
                *seed ^= *seed << 17;
                items.swap(i, (*seed as usize) % (i + 1));
            }
        }
        for case in 1..=64_u64 {
            let mut seed = case;
            let inventories: Vec<Vec<String>> = (0..3)
                .map(|group| {
                    (0..(4 + case as usize % 9))
                        .map(|i| format!("group-{group}-{i}"))
                        .collect()
                })
                .collect();
            let hidden: Vec<String> = (0..7)
                .map(|i| format!("hidden-stale-foreign-{i}"))
                .collect();
            let mut stored: Vec<String> = inventories
                .iter()
                .flat_map(|group| group.iter().take(group.len() - 2).cloned())
                .chain(hidden.clone())
                .collect();
            shuffle(&mut stored, &mut seed);
            for step in 0..24 {
                let group = &inventories[step % inventories.len()];
                // Scope changes between saves, including IDs omitted by older drafts.
                let target: HashSet<String> = group
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| (i + step) % 4 != 0)
                    .map(|(_, id)| id.clone())
                    .collect();
                let mut submitted: Vec<String> = group
                    .iter()
                    .enumerate()
                    .filter(|(i, id)| target.contains(*id) && (i + step) % 3 != 0)
                    .map(|(_, id)| id.clone())
                    .collect();
                shuffle(&mut submitted, &mut seed);
                if step % 5 == 0 {
                    submitted.clear();
                }
                let before = stored.clone();
                stored = merge_group(&before, &submitted, &target);
                let protected = |ids: &[String]| {
                    ids.iter()
                        .filter(|id| !target.contains(*id))
                        .cloned()
                        .collect::<Vec<_>>()
                };
                assert_eq!(
                    protected(&stored),
                    protected(&before),
                    "case {case} step {step}"
                );
                assert_eq!(
                    stored
                        .iter()
                        .filter(|id| hidden.contains(id))
                        .cloned()
                        .collect::<Vec<_>>(),
                    before
                        .iter()
                        .filter(|id| hidden.contains(id))
                        .cloned()
                        .collect::<Vec<_>>()
                );
                let logical: Vec<String> = stored
                    .iter()
                    .filter(|id| target.contains(*id))
                    .cloned()
                    .collect();
                if submitted.is_empty() {
                    assert!(logical.is_empty());
                } else {
                    assert!(logical.starts_with(&submitted));
                    assert_eq!(
                        &logical[submitted.len()..],
                        before
                            .iter()
                            .filter(|id| target.contains(*id) && !submitted.contains(*id))
                            .cloned()
                            .collect::<Vec<_>>()
                    );
                    if let Some(last) = before.iter().rposition(|id| target.contains(id)) {
                        assert!(
                            stored.ends_with(&before[last + 1..]),
                            "new IDs must precede the untouched suffix"
                        );
                    }
                }
                assert_eq!(
                    merge_group(&stored, &logical, &target),
                    stored,
                    "logical resubmission must be a no-op"
                );
                assert_eq!(stored.iter().collect::<HashSet<_>>().len(), stored.len());
                assert!(
                    stored
                        .iter()
                        .all(|id| before.contains(id) || submitted.contains(id))
                );
                assert!(
                    logical.iter().all(|id| group.contains(id)),
                    "IDs cannot move across immutable groups"
                );
            }
        }
    }
}

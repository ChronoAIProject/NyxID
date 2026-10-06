use std::collections::{HashMap, HashSet};

use mongodb::{Database, bson::doc, options::ReturnDocument};
use uuid::Uuid;

use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::service_preference::{COLLECTION_NAME, ServicePreference},
    services::{
        platform_key_service,
        unified_key_service::{self, KeyView},
    },
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

pub fn resolve_visible(ordered: &[String], visible: &HashSet<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    ordered
        .iter()
        .filter(|id| visible.contains(*id) && seen.insert(*id))
        .cloned()
        .collect()
}

pub fn rank_map(ordered: &[String], visible: &HashSet<String>) -> HashMap<String, u32> {
    resolve_visible(ordered, visible)
        .into_iter()
        .enumerate()
        .map(|(index, id)| (id, (index + 1) as u32))
        .collect()
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

pub async fn visible_inventory(
    db: &Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    scope: Option<&[String]>,
    api_key: bool,
) -> AppResult<Vec<KeyView>> {
    let providers = platform_key_service::load_providers(db).await?;
    let grants = platform_key_service::OwnerGrants::load_for_listing(db, user_id).await?;
    let views = unified_key_service::list_keys_read_only_with_grants(
        db,
        encryption_keys,
        user_id,
        &grants,
        &providers,
    )
    .await?;
    Ok(filter_inventory(views, scope, api_key))
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
        return Err(AppError::ValidationError(
            "preference order exceeds 200 services".into(),
        ));
    }
    let mut seen = HashSet::new();
    for id in ordered {
        if !Uuid::parse_str(id).is_ok_and(|uuid| {
            uuid.get_version_num() == 4
                && uuid.get_variant() == uuid::Variant::RFC4122
                && uuid.to_string() == *id
        }) {
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

pub struct Replacement {
    pub preference: Option<ServicePreference>,
    pub changed: bool,
}

pub async fn replace(
    db: &Database,
    user_id: &str,
    ordered: &[String],
    expected_version: i64,
    visible: &HashSet<String>,
) -> AppResult<Replacement> {
    validate_order(ordered, expected_version, visible)?;
    let collection = db.collection::<ServicePreference>(COLLECTION_NAME);
    let current = get(db, user_id).await?;
    if current.as_ref().map_or(0, |row| row.version) != expected_version {
        return Err(AppError::Conflict(
            "preference order changed elsewhere".into(),
        ));
    }
    if current
        .as_ref()
        .map_or(&[][..], |row| row.ordered.as_slice())
        == ordered
    {
        return Ok(Replacement {
            preference: current,
            changed: false,
        });
    }
    let now = bson::DateTime::now().to_chrono();
    let preference = if expected_version == 0 && current.is_none() {
        let row = ServicePreference {
            user_id: user_id.into(),
            ordered: ordered.into(),
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
        collection.find_one_and_update(
            filter,
            doc! { "$set": { "ordered": ordered, "updated_at": bson::DateTime::from_chrono(now) }, "$inc": { "version": 1_i64 } },
        ).return_document(ReturnDocument::After).await?
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

pub async fn order_discovery(
    db: &Database,
    user_id: &str,
    mut services: Vec<super::mcp_service::McpToolService>,
) -> AppResult<(
    Vec<super::mcp_service::McpToolService>,
    HashMap<String, u32>,
)> {
    let visible = services
        .iter()
        .filter(|service| service.source.is_user_service())
        .map(|service| service.service_id.clone())
        .collect();
    let preference = get(db, user_id).await?;
    let ranks = rank_map(
        preference
            .as_ref()
            .map_or(&[], |row| row.ordered.as_slice()),
        &visible,
    );
    super::mcp_service::order_services_by_preference(&mut services, &ranks);
    Ok((services, ranks))
}

pub async fn detail_rank(
    db: &Database,
    user_id: &str,
    service_id: &str,
    scope: Option<&[String]>,
    api_key: bool,
) -> AppResult<Option<u32>> {
    let Some(preference) = get(db, user_id).await? else {
        return Ok(None);
    };
    if !preference.ordered.iter().any(|id| id == service_id) {
        return Ok(None);
    }
    let selected: Vec<_> = preference
        .ordered
        .iter()
        .take(MAX_ORDERED_SERVICES)
        .filter(|id| scope.is_none_or(|scope| scope.contains(id)))
        .cloned()
        .collect();
    let grants = platform_key_service::OwnerGrants::load_for_listing(db, user_id).await?;
    let tagged = super::user_service_service::list_user_services_with_sources_selected(
        db,
        user_id,
        false,
        true,
        grants.memberships(),
        Some(&selected),
    )
    .await?;
    let tagged: Vec<_> = tagged
        .into_iter()
        .filter(|row| !api_key || !row.source.is_viewer_org())
        .collect();
    let endpoint_ids: Vec<_> = tagged.iter().map(|row| &row.service.endpoint_id).collect();
    use futures::TryStreamExt;
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
    let visible = tagged
        .iter()
        .filter(|row| endpoints.contains(row.service.endpoint_id.as_str()))
        .map(|row| row.service.id.clone())
        .collect();
    Ok(rank_map(&preference.ordered, &visible)
        .get(service_id)
        .copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_preference_validation_and_dense_visibility() {
        let a = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string();
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
            assert!(matches!(
                validate_order(&ids, version, &visible),
                Err(AppError::ValidationError(_))
            ));
        }
        let non_rfc = "aaaaaaaa-aaaa-4aaa-1aaa-aaaaaaaaaaaa".to_string();
        assert!(
            validate_order(
                std::slice::from_ref(&non_rfc),
                0,
                &HashSet::from([non_rfc.clone()])
            )
            .unwrap_err()
            .to_string()
            .contains("canonical UUID v4")
        );
        let foreign = validate_order(std::slice::from_ref(&b), 0, &visible)
            .unwrap_err()
            .to_string();
        assert_eq!(
            foreign,
            validate_order(&[Uuid::new_v4().to_string()], 0, &visible)
                .unwrap_err()
                .to_string()
        );
        assert_eq!(rank_map(&[b, a.clone()], &visible), HashMap::from([(a, 1)]));
        validate_order(&[], MAX_EXPECTED_VERSION, &visible).unwrap();
    }
}

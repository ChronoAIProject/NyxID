//! Idempotent URL maintenance; aliases do not alter endpoint authority or fetch remote specs.
use crate::{config::AppConfig, errors::AppResult};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{Document, doc},
};

pub(crate) fn normalized_hosted_spec(config: &AppConfig, value: &str) -> Option<String> {
    let canonical = config.canonical_base_url(value);
    if canonical == value {
        return None;
    }
    let path = canonical.strip_prefix(config.base_url.trim_end_matches('/'))?;
    // Registered exact path only: no query, fragment, encoded path, unknown key, or extra slash.
    super::catalog_spec_registry::spec_for_url_path(path)?;
    Some(canonical.into_owned())
}

pub async fn normalize_hosted_specs(db: &Database, config: &AppConfig) -> AppResult<u64> {
    if config.domain_aliases.base_urls.is_empty() {
        return Ok(0);
    }
    let mut changed = 0;
    for collection in [
        crate::models::downstream_service::COLLECTION_NAME,
        crate::models::user_endpoint::COLLECTION_NAME,
    ] {
        let rows = db.collection::<Document>(collection);
        let mut cursor = rows
            .find(doc! {"openapi_spec_url": {"$type":"string"}})
            .projection(doc! {"openapi_spec_url":1})
            .batch_size(100)
            .await?;
        while let Some(row) = cursor.try_next().await? {
            let (Ok(value), Some(id)) = (row.get_str("openapi_spec_url"), row.get("_id")) else {
                continue;
            };
            if let Some(canonical) = normalized_hosted_spec(config, value) {
                changed += rows
                    .update_one(
                        doc! {"_id":id,"openapi_spec_url":value},
                        doc! {"$set":{"openapi_spec_url":canonical}},
                    )
                    .await?
                    .modified_count;
            }
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn domain_alias_hosted_spec_normalization_is_exact_idempotent_and_opt_in() {
        let db = crate::test_utils::connect_test_database("domain_alias_specs")
            .await
            .unwrap();
        let mut config = crate::test_utils::test_app_config();
        config.base_url = "https://new.example".into();
        let path = super::super::catalog_spec_registry::spec_path_for_slug("api-github").unwrap();
        let old = format!("https://old.example{path}");
        let canonical = format!("https://new.example{path}");
        let unchanged = [
            canonical.clone(),
            format!("https://foreign.example{path}"),
            format!("{old}?v=1"),
            format!("{old}#fragment"),
            format!("{old}/"),
            "https://old.example/api/v1/catalog-specs/unknown.openapi.json".into(),
            format!("https://old.example.evil{path}"),
        ];
        for name in [
            crate::models::downstream_service::COLLECTION_NAME,
            crate::models::user_endpoint::COLLECTION_NAME,
        ] {
            let rows = db.collection::<Document>(name);
            rows.insert_one(doc! {"_id":"migrate", "openapi_spec_url":&old, "untouched":true})
                .await
                .unwrap();
            for (index, url) in unchanged.iter().enumerate() {
                rows.insert_one(doc! {"_id":index.to_string(), "openapi_spec_url":url})
                    .await
                    .unwrap();
            }
        }
        assert_eq!(normalize_hosted_specs(&db, &config).await.unwrap(), 0);
        config.domain_aliases.base_urls = vec!["https://old.example".into()];
        assert_eq!(normalize_hosted_specs(&db, &config).await.unwrap(), 2);
        assert_eq!(normalize_hosted_specs(&db, &config).await.unwrap(), 0);
        for name in [
            crate::models::downstream_service::COLLECTION_NAME,
            crate::models::user_endpoint::COLLECTION_NAME,
        ] {
            let rows = db.collection::<Document>(name);
            let migrated = rows
                .find_one(doc! {"_id":"migrate"})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(migrated.get_str("openapi_spec_url").unwrap(), canonical);
            assert!(migrated.get_bool("untouched").unwrap());
            for (index, url) in unchanged.iter().enumerate() {
                let row = rows
                    .find_one(doc! {"_id":index.to_string()})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(row.get_str("openapi_spec_url").unwrap(), url);
            }
        }
    }
}

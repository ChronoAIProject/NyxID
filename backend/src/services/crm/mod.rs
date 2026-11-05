pub mod activities;
pub mod fields;
pub mod generation;
pub mod links;
pub mod objects;
pub mod people;
pub mod query;
pub mod records;
pub mod tools;
pub mod transfer;
pub mod views;

use crate::errors::{AppError, AppResult, CrmValidationIssue};

pub const FLAG_KEY: &str = "cms:poc-v1";

pub async fn enabled(db: &mongodb::Database, actor: &str) -> AppResult<bool> {
    crate::services::feature_flag_service::personal_flag_enabled(db, actor, FLAG_KEY).await
}

pub async fn require_enabled(db: &mongodb::Database, actor: &str) -> AppResult<()> {
    if enabled(db, actor).await? {
        Ok(())
    } else {
        Err(AppError::NotFound("CRM resource not found".into()))
    }
}

/// Remove additive CRM fields at non-CRM response boundaries for flag-off callers.
pub fn hide_fields(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.remove("crm");
            map.remove("crm_artifacts");
            for child in map.values_mut() {
                hide_fields(child);
            }
        }
        serde_json::Value::Array(rows) => {
            for row in rows {
                hide_fields(row);
            }
        }
        _ => {}
    }
}

pub(crate) async fn fence_owner(
    db: &mongodb::Database,
    owner_id: &str,
    tx: &mut mongodb::ClientSession,
) -> AppResult<()> {
    crate::services::erasure_admission::fence_in_session(db, owner_id, tx).await?;
    tools::fence_admission(db, owner_id, tx).await
}

pub async fn ensure_indexes(db: &mongodb::Database) -> mongodb::error::Result<()> {
    use mongodb::{
        IndexModel,
        bson::{Document, doc},
        options::IndexOptions,
    };
    use std::time::Duration;

    db.collection::<Document>(crate::models::crm_object::COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"owner_id": 1, "key": 1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<Document>(crate::models::crm_record::COLLECTION_NAME)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"owner_id": 1, "object_key": 1, "archived_at": 1, "_id": 1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"owner_id": 1, "object_id": 1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"owner_id": 1, "object_key": 1, "refs.company": 1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"owner_id": 1, "object_key": 1, "refs.related": 1})
                .build(),
        ])
        .await?;
    db.collection::<Document>(crate::models::crm_link::COLLECTION_NAME)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"source_record_id": 1, "field_key": 1})
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! {"single": true})
                        .build(),
                )
                .build(),
            IndexModel::builder()
                .keys(doc! {"source_record_id": 1, "field_key": 1, "target_record_id": 1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! {"target_record_id": 1, "field_key": 1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"owner_id": 1, "source_object": 1})
                .build(),
        ])
        .await?;
    db.collection::<Document>(crate::models::crm_activity::COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"owner_id": 1, "record_id": 1, "created_at": -1, "_id": -1})
                .build(),
        )
        .await?;
    db.collection::<Document>(crate::models::crm_saved_view::COLLECTION_NAME)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"owner_id": 1, "updated_at": -1, "_id": -1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"deleted_at": 1})
                .options(
                    IndexOptions::builder()
                        .expire_after(Duration::from_secs(7 * 24 * 60 * 60))
                        .build(),
                )
                .build(),
        ])
        .await?;
    db.collection::<Document>(crate::models::crm_mutation_receipt::COLLECTION_NAME)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"expires_at": 1})
                .options(IndexOptions::builder().expire_after(Duration::ZERO).build())
                .build(),
            IndexModel::builder().keys(doc! {"owner_id": 1}).build(),
        ])
        .await?;
    db.collection::<Document>("crm_generation_jobs")
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"expires_at": 1})
                .options(IndexOptions::builder().expire_after(Duration::ZERO).build())
                .build(),
            IndexModel::builder().keys(doc! {"owner_id": 1}).build(),
        ])
        .await?;
    Ok(())
}

pub fn validation_error(path: impl Into<String>, message: impl Into<String>) -> AppError {
    AppError::CrmValidationFailed(vec![CrmValidationIssue {
        path: path.into(),
        message: message.into(),
    }])
}

pub fn validation_result(errors: Vec<CrmValidationIssue>) -> AppResult<()> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(AppError::CrmValidationFailed(errors))
    }
}

#[cfg(test)]
mod regression_tests;

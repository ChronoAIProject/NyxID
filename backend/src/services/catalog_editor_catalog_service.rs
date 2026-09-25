use futures::TryStreamExt;
use mongodb::{Database, bson::doc};
use serde::Deserialize;
use uuid::Uuid;

use crate::{
    errors::{AppError, AppResult},
    models::{catalog_skill_revision::SkillState, downstream_service::COLLECTION_NAME},
};

/// Catalog fields visible to an editor. Queries never load credentials or instance data.
#[derive(Deserialize)]
pub struct CatalogMetadata {
    #[serde(rename = "_id")]
    pub id: String,
    pub slug: String,
    pub name: String,
    #[serde(default = "http_service_type")]
    pub service_type: String,
    pub is_active: bool,
    #[serde(flatten)]
    pub skills: SkillState,
    #[serde(default)]
    pub skills_revision: i64,
}

fn http_service_type() -> String {
    "http".into()
}

fn projection() -> mongodb::bson::Document {
    doc! {"_id": 1, "slug": 1, "name": 1, "service_type": 1, "is_active": 1,
    "recommended_skills": 1, "recommended_skill_refs": 1, "skills_revision": 1}
}

pub async fn list(db: &Database) -> AppResult<Vec<CatalogMetadata>> {
    Ok(db
        .collection::<CatalogMetadata>(COLLECTION_NAME)
        .find(doc! {})
        .projection(projection())
        .sort(doc! {"_id": 1})
        .await?
        .try_collect()
        .await?)
}

pub async fn read(db: &Database, id: &str) -> AppResult<CatalogMetadata> {
    if id.len() != 36 || Uuid::parse_str(id).is_err() {
        return Err(AppError::NotFound("Catalog service not found".into()));
    }
    db.collection::<CatalogMetadata>(COLLECTION_NAME)
        .find_one(doc! {"_id": id})
        .projection(projection())
        .await?
        .ok_or_else(|| AppError::NotFound("Catalog service not found".into()))
}

//! Metadata-only inventory for model catalog identities. Never decrypts a credential.
use crate::{
    errors::{AppError, AppResult},
    models::{downstream_service::COLLECTION_NAME, platform_model},
};
use bson::{Document, doc};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::Database;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    GcpServiceAccount,
    Openai,
    Anthropic,
}
impl Kind {
    pub fn base_url(self) -> &'static str {
        match self {
            Self::GcpServiceAccount => "https://aiplatform.googleapis.com",
            Self::Openai => "https://api.openai.com",
            Self::Anthropic => "https://api.anthropic.com",
        }
    }
    pub fn from_url(url: &str) -> Option<Self> {
        [Self::GcpServiceAccount, Self::Openai, Self::Anthropic]
            .into_iter()
            .find(|kind| url.trim_end_matches('/') == kind.base_url())
    }
    pub fn validate(self, raw: &str) -> AppResult<()> {
        if raw.len() > 64 * 1024 {
            return Err(AppError::ValidationError(
                "Credential exceeds 64 KiB".into(),
            ));
        }
        super::platform_model_execution::token::validate_saved_credential(
            raw,
            self == Self::GcpServiceAccount,
        )
    }
}

#[derive(Deserialize)]
pub struct Record {
    #[serde(rename = "_id")]
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub is_active: bool,
    pub platform_key: crate::models::downstream_service::PlatformKeyConfig,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
fn projection() -> Document {
    doc! {"name":1,"base_url":1,"is_active":1,"platform_key":1,"created_at":1,"updated_at":1}
}
pub async fn list(db: &Database, offering: bool, after: Option<&str>) -> AppResult<Vec<Record>> {
    let mut filter =
        doc! {"platform_key.purpose": if offering {"model_offering"} else {"model_route"}};
    if let Some(after) = after {
        filter.insert("_id", doc! {"$gt":after});
    }
    Ok(db
        .collection::<Record>(COLLECTION_NAME)
        .find(filter)
        .projection(projection())
        .sort(doc! {"_id":1})
        .limit(100)
        .await?
        .try_collect()
        .await?)
}
pub async fn get(db: &Database, id: &str) -> AppResult<Record> {
    db.collection::<Record>(COLLECTION_NAME)
        .find_one(doc! {"_id":id,"platform_key.purpose":"model_route"})
        .projection(projection())
        .await?
        .ok_or_else(|| AppError::NotFound("Provider credential not found".into()))
}
/// Join only current routing revisions. Retired historical revisions are not live references.
pub async fn references(db: &Database, ids: &[String], offering: bool) -> AppResult<Vec<Document>> {
    if offering {
        return Ok(db.collection::<Document>(platform_model::COLLECTION_NAME).aggregate(vec![
            doc! {"$match":{"billing_service_id":{"$in":ids}}},
            doc! {"$project":{"_id":0,"credential_id":"$billing_service_id","model_id":"$_id","public_id":1,"display_name":1,"enabled":1}}
        ]).await?.try_collect().await?);
    }
    Ok(db.collection::<Document>(platform_model::ROUTING_COLLECTION).aggregate(vec![
        doc! {"$match":{"routes.credential_service_id":{"$in":ids}}},
        doc! {"$lookup":{"from":platform_model::COLLECTION_NAME,"localField":"model_id","foreignField":"_id","as":"model"}},
        doc! {"$unwind":"$model"},
        doc! {"$match":{"$expr":{"$eq":["$_id","$model.routing_revision_id"]}}},
        doc! {"$unwind":"$routes"},
        doc! {"$match":{"routes.credential_service_id":{"$in":ids}}},
        doc! {"$project":{"_id":0,"credential_id":"$routes.credential_service_id","model_id":"$model._id","public_id":"$model.public_id","display_name":"$model.display_name","route_id":"$routes.id","enabled":"$routes.enabled"}},
        doc! {"$sort":{"public_id":1,"route_id":1}},
    ]).await?.try_collect().await?)
}

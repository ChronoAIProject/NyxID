use chrono::{DateTime, Utc};
use mongodb::{
    ClientSession, Database,
    bson::{self, Bson, Document, doc},
    results::{InsertOneResult, UpdateResult},
};

use crate::{
    errors::{AppError, AppResult},
    models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS},
};

const AUTHORITY_FIELDS: [&str; 2] = ["state_version", "updated_at"];

fn operator_document<'a>(update: &'a mut Document, operator: &str) -> AppResult<&'a mut Document> {
    if !update.contains_key(operator) {
        update.insert(operator, Document::new());
    }
    update
        .get_document_mut(operator)
        .map_err(|_| AppError::Internal(format!("API key {operator} must be a document")))
}

fn reject_authority_override(update: &Document) -> AppResult<()> {
    for (operator, value) in update {
        if !operator.starts_with('$') {
            return Err(AppError::Internal(
                "API key mutations must use update operators".to_string(),
            ));
        }
        let Bson::Document(fields) = value else {
            return Err(AppError::Internal(format!(
                "API key {operator} must be a document"
            )));
        };
        if fields
            .keys()
            .any(|field| AUTHORITY_FIELDS.contains(&field.as_str()))
        {
            return Err(AppError::Internal(
                "API key authority fields are owned by the mutation service".to_string(),
            ));
        }
        if operator == "$rename"
            && fields.values().any(|value| {
                value
                    .as_str()
                    .is_some_and(|field| AUTHORITY_FIELDS.contains(&field))
            })
        {
            return Err(AppError::Internal(
                "API key authority fields cannot be rename targets".to_string(),
            ));
        }
    }
    Ok(())
}

/// Add the authoritative version/timestamp transition to an ApiKey update.
///
/// Missing legacy `state_version` fields are initialized to one by MongoDB's
/// `$inc` semantics. Callers cannot supply either authority field themselves.
pub fn authoritative_update_at(
    mut update: Document,
    updated_at: DateTime<Utc>,
) -> AppResult<Document> {
    reject_authority_override(&update)?;
    operator_document(&mut update, "$set")?.insert(
        "updated_at",
        Bson::DateTime(bson::DateTime::from_chrono(updated_at)),
    );
    operator_document(&mut update, "$inc")?.insert("state_version", 1_i64);
    Ok(update)
}

pub fn authoritative_update(update: Document) -> AppResult<Document> {
    authoritative_update_at(update, Utc::now())
}

pub async fn update_one(
    db: &Database,
    filter: Document,
    update: Document,
    session: Option<&mut ClientSession>,
) -> AppResult<UpdateResult> {
    let collection = db.collection::<ApiKey>(API_KEYS);
    let action = collection.update_one(filter, authoritative_update(update)?);
    Ok(match session {
        Some(session) => action.session(session).await?,
        None => action.await?,
    })
}

pub async fn insert_one(
    db: &Database,
    key: &ApiKey,
    session: Option<&mut ClientSession>,
) -> AppResult<InsertOneResult> {
    if key.state_version <= 0 || key.updated_at.is_none() {
        return Err(AppError::Internal(
            "new API keys require positive authority evidence".to_string(),
        ));
    }
    let collection = db.collection::<ApiKey>(API_KEYS);
    let action = collection.insert_one(key);
    Ok(match session {
        Some(session) => action.session(session).await?,
        None => action.await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authoritative_update_preserves_business_operators_and_adds_authority() {
        let now = Utc::now();
        let update = authoritative_update_at(
            doc! {
                "$set": { "is_active": false },
                "$pull": { "allowed_service_ids": "svc-1" },
            },
            now,
        )
        .unwrap();

        assert_eq!(update["$set"]["is_active"], Bson::Boolean(false));
        assert_eq!(
            update["$pull"]["allowed_service_ids"],
            Bson::String("svc-1".to_string())
        );
        assert_eq!(update["$inc"]["state_version"], Bson::Int64(1));
        assert_eq!(
            update["$set"]["updated_at"],
            Bson::DateTime(bson::DateTime::from_chrono(now))
        );
    }

    #[test]
    fn authoritative_update_rejects_authority_overrides() {
        for update in [
            doc! { "$set": { "updated_at": bson::DateTime::now() } },
            doc! { "$inc": { "state_version": 10 } },
            doc! { "$rename": { "legacy_version": "state_version" } },
        ] {
            assert!(authoritative_update(update).is_err());
        }
    }

    #[test]
    fn authoritative_update_rejects_replacement_documents() {
        assert!(authoritative_update(doc! { "is_active": false }).is_err());
    }
}

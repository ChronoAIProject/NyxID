use crate::errors::AppResult;
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{Document, doc},
};
use serde::Serialize;
#[derive(Debug, Serialize)]
pub struct Person {
    pub id: String,
    pub label: String,
    pub available: bool,
}
#[derive(Debug, Serialize)]
pub struct PeoplePage {
    pub people: Vec<Person>,
    pub next_cursor: Option<String>,
}

pub async fn get(db: &Database, owner: &str, id: &str) -> AppResult<Person> {
    let eligible = id == owner || db.collection::<Document>(crate::models::org_membership::COLLECTION_NAME).find_one(doc! {"org_user_id":owner,"member_user_id":id,"revoked_at":mongodb::bson::Bson::Null}).await?.is_some();
    let person = if eligible {
        db.collection::<Document>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id":id,"is_active":true,"user_type":"person"})
            .projection(doc! {"display_name":1})
            .await?
    } else {
        None
    };
    Ok(Person {
        id: id.into(),
        label: person
            .as_ref()
            .and_then(|p| p.get_str("display_name").ok())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(if person.is_some() {
                "Member"
            } else {
                "Unavailable member"
            })
            .into(),
        available: person.is_some(),
    })
}
pub async fn list(db: &Database, owner: &str, cursor: Option<&str>) -> AppResult<PeoplePage> {
    if let Some(cursor) = cursor {
        uuid::Uuid::parse_str(cursor)
            .map_err(|_| super::validation_error("cursor", "is invalid"))?;
    }
    let is_person = db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id":owner,"user_type":"person","is_active":true})
        .await?
        .is_some();
    if is_person {
        return Ok(PeoplePage {
            people: if cursor.is_none() {
                vec![get(db, owner, owner).await?]
            } else {
                vec![]
            },
            next_cursor: None,
        });
    }
    let mut filter = doc! {"org_user_id":owner,"revoked_at":mongodb::bson::Bson::Null};
    if let Some(cursor) = cursor {
        filter.insert("member_user_id", doc! {"$gt":cursor});
    }
    // One bounded batch lookup; names of unrelated people are never queried.
    let mut rows: Vec<Document> = db
        .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"member_user_id":1})
        .limit(51)
        .max_time(std::time::Duration::from_secs(5))
        .await?
        .try_collect()
        .await?;
    let more = rows.len() > 50;
    rows.truncate(50);
    let ids: Vec<String> = rows
        .iter()
        .filter_map(|r| r.get_str("member_user_id").ok().map(str::to_owned))
        .collect();
    let people: Vec<Document> = db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .find(doc! {"_id":{"$in":&ids},"is_active":true,"user_type":"person"})
        .projection(doc! {"display_name":1})
        .await?
        .try_collect()
        .await?;
    let result = ids
        .iter()
        .map(|id| {
            let person = people.iter().find(|p| p.get_str("_id").ok() == Some(id));
            Person {
                id: id.clone(),
                label: person
                    .and_then(|p| p.get_str("display_name").ok())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or(if person.is_some() {
                        "Member"
                    } else {
                        "Unavailable member"
                    })
                    .into(),
                available: person.is_some(),
            }
        })
        .collect();
    Ok(PeoplePage {
        people: result,
        next_cursor: more.then(|| ids.last().cloned()).flatten(),
    })
}

/// Resolve only active members of this authorized owner in two bounded reads.
pub async fn lookup(db: &Database, owner: &str, ids: &[String]) -> AppResult<Vec<Person>> {
    if ids.len() > 200 || ids.iter().any(|id| uuid::Uuid::parse_str(id).is_err()) {
        return Err(super::validation_error(
            "ids",
            "at most 200 UUIDs are allowed",
        ));
    }
    let mut eligible = std::collections::HashSet::from([owner.to_owned()]);
    let memberships: Vec<Document> = db.collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
        .find(doc! {"org_user_id":owner,"member_user_id":{"$in":ids},"revoked_at":mongodb::bson::Bson::Null})
        .projection(doc! {"member_user_id":1}).limit(200).max_time(std::time::Duration::from_secs(5)).await?.try_collect().await?;
    eligible.extend(
        memberships
            .iter()
            .filter_map(|row| row.get_str("member_user_id").ok().map(str::to_owned)),
    );
    let allowed: Vec<&String> = ids.iter().filter(|id| eligible.contains(*id)).collect();
    let people: Vec<Document> = db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .find(doc! {"_id":{"$in":allowed},"is_active":true,"user_type":"person"})
        .projection(doc! {"display_name":1})
        .limit(200)
        .max_time(std::time::Duration::from_secs(5))
        .await?
        .try_collect()
        .await?;
    Ok(ids
        .iter()
        .map(|id| {
            let person = people
                .iter()
                .find(|p| p.get_str("_id").ok() == Some(id.as_str()));
            Person {
                id: id.clone(),
                available: person.is_some(),
                label: person
                    .and_then(|p| p.get_str("display_name").ok())
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or(if person.is_some() {
                        "Member"
                    } else {
                        "Unavailable member"
                    })
                    .into(),
            }
        })
        .collect())
}

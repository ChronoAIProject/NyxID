//! Identity lookup for the admin Usage filter pickers. Returns only the
//! id/label/detail a picker renders. Admin readers search the whole
//! directory, as they could through the admin user and service-account lists.
//! Usage-only readers see just identities recorded as an actor or owner in the
//! usage rollups, the report's eligible usage, so the lookup never discloses
//! more than the usage report itself. Usage appears there once rolled up
//! (the rollup sweep runs at least every minute).
use std::collections::HashSet;
use std::time::Duration;

use futures::TryStreamExt;
use mongodb::bson::{Bson, Document, doc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use super::{QUERY_TIMEOUT, query_error};
use crate::errors::{AppError, AppResult};
use crate::models::service_account::COLLECTION_NAME as SERVICE_ACCOUNTS;
use crate::models::user::COLLECTION_NAME as USERS;
use crate::services::reporting_identity_service::{self, ReportingIdentity, nonempty_field};

const PAGE_SIZE: i64 = 50;
pub const MAX_IDS: usize = 100;
const ROLLUPS: [&str; 2] = [
    crate::models::usage_rollup_hourly::COLLECTION_NAME,
    crate::models::usage_rollup_daily::COLLECTION_NAME,
];

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct IdentityQuery {
    /// `actors` (people and service accounts) or `owners` (also orgs).
    pub kind: Option<String>,
    /// Case-insensitive substring; people match email, orgs match
    /// email/name/slug, service accounts match name.
    pub search: Option<String>,
    /// Comma-separated identity ids (at most 100) to label instead of searching.
    pub ids: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IdentityOption {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct IdentityOptionsResponse {
    pub options: Vec<IdentityOption>,
    pub total: u64,
}

/// Which identities the caller may discover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityScope {
    /// Admins and Operators: the full directory.
    Directory,
    /// Usage-only readers: identities recorded in the usage rollups.
    Usage,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum IdentityKind {
    Actors,
    Owners,
}

impl IdentityKind {
    /// The rollup field that records this kind of identity.
    fn rollup_field(self) -> &'static str {
        match self {
            Self::Actors => "actor",
            Self::Owners => "owner",
        }
    }
}

pub async fn lookup(
    db: &mongodb::Database,
    query: IdentityQuery,
    scope: IdentityScope,
) -> AppResult<IdentityOptionsResponse> {
    // A wall-clock bound over every query, as for the usage report itself.
    tokio::time::timeout(
        QUERY_TIMEOUT + Duration::from_secs(2),
        lookup_inner(db, query, scope),
    )
    .await
    .map_err(|_| AppError::AdminUsageQueryTimeout)?
}

async fn lookup_inner(
    db: &mongodb::Database,
    query: IdentityQuery,
    scope: IdentityScope,
) -> AppResult<IdentityOptionsResponse> {
    if let Some(ids) = query.ids.as_deref() {
        return label_ids(db, ids, scope).await;
    }
    let kind = match query.kind.as_deref() {
        Some("actors") => IdentityKind::Actors,
        Some("owners") => IdentityKind::Owners,
        _ => {
            return Err(AppError::BadRequest(
                "kind must be actors or owners".to_string(),
            ));
        }
    };
    let pattern = query
        .search
        .as_deref()
        .filter(|search| !search.is_empty())
        .map(|search| doc! { "$regex": regex::escape(search), "$options": "i" });

    let mut person_filter = doc! { "user_type": { "$ne": "org" } };
    let mut org_filter = doc! { "user_type": "org" };
    let mut account_filter = doc! {};
    if let Some(pattern) = pattern {
        person_filter.insert("email", pattern.clone());
        org_filter.insert(
            "$or",
            vec![
                doc! { "email": pattern.clone() },
                doc! { "display_name": pattern.clone() },
                doc! { "slug": pattern.clone() },
            ],
        );
        account_filter.insert("name", pattern);
    }
    let usage_field = (scope == IdentityScope::Usage).then(|| kind.rollup_field());

    let orgs = async {
        if kind == IdentityKind::Owners {
            search_collection(db, USERS, org_filter, usage_field).await
        } else {
            Ok((Vec::new(), 0))
        }
    };
    let people = search_collection(db, USERS, person_filter, usage_field);
    let accounts = search_collection(db, SERVICE_ACCOUNTS, account_filter, usage_field);
    let (orgs, people, accounts) = tokio::try_join!(orgs, people, accounts)?;

    let mut response = IdentityOptionsResponse {
        options: Vec::new(),
        total: 0,
    };
    for (options, total) in [orgs, people, accounts] {
        response.options.extend(options);
        response.total += total;
    }
    Ok(response)
}

/// One page of matching identities from `collection` plus the total match
/// count. With `usage_field`, only identities that the usage rollups record
/// under that field match; each check is an indexed existence lookup.
async fn search_collection(
    db: &mongodb::Database,
    collection: &str,
    filter: Document,
    usage_field: Option<&str>,
) -> AppResult<(Vec<IdentityOption>, u64)> {
    let mut pipeline = vec![doc! { "$match": filter }];
    if let Some(field) = usage_field {
        for (index, rollup) in ROLLUPS.iter().enumerate() {
            pipeline.push(doc! { "$lookup": {
                "from": *rollup,
                "localField": "_id",
                "foreignField": field,
                "pipeline": [{ "$limit": 1 }, { "$project": { "_id": 1 } }],
                "as": format!("usage_{index}"),
            } });
        }
        pipeline.push(doc! { "$match": { "$or": [
            { "usage_0": { "$ne": [] } },
            { "usage_1": { "$ne": [] } },
        ] } });
    }
    pipeline.push(doc! { "$facet": {
        "page": [
            { "$sort": { "created_at": -1, "_id": 1 } },
            { "$limit": PAGE_SIZE },
            { "$project": { "name": 1, "email": 1, "display_name": 1, "user_type": 1 } },
        ],
        "total": [{ "$count": "n" }],
    } });

    let result: Option<Document> = db
        .collection::<Document>(collection)
        .aggregate(pipeline)
        .max_time(QUERY_TIMEOUT)
        .await
        .map_err(query_error)?
        .try_next()
        .await
        .map_err(query_error)?;
    let Some(result) = result else {
        return Ok((Vec::new(), 0));
    };
    let total = result
        .get_array("total")
        .ok()
        .and_then(|rows| rows.first())
        .and_then(Bson::as_document)
        .and_then(|row| match row.get("n") {
            Some(Bson::Int32(n)) => u64::try_from(*n).ok(),
            Some(Bson::Int64(n)) => u64::try_from(*n).ok(),
            _ => None,
        })
        .unwrap_or(0);
    let accounts = collection == SERVICE_ACCOUNTS;
    let options = result
        .get_array("page")
        .map(|rows| {
            rows.iter()
                .filter_map(Bson::as_document)
                .filter_map(|row| {
                    let id = row.get_str("_id").ok()?.to_string();
                    let identity = if accounts {
                        ReportingIdentity {
                            display_name: nonempty_field(row, "name"),
                            email: None,
                            user_type: "service_account".into(),
                        }
                    } else {
                        ReportingIdentity {
                            display_name: nonempty_field(row, "display_name"),
                            email: nonempty_field(row, "email"),
                            user_type: row.get_str("user_type").unwrap_or("person").into(),
                        }
                    };
                    Some(option(id, &identity))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok((options, total))
}

/// The subset of `ids` recorded as an actor or owner in the usage rollups;
/// every query is an indexed `$in` over at most [`MAX_IDS`] ids.
async fn ids_with_usage(db: &mongodb::Database, ids: &[String]) -> AppResult<HashSet<String>> {
    let mut found = HashSet::new();
    for rollup in ROLLUPS {
        for field in ["actor", "owner"] {
            let values = db
                .collection::<Document>(rollup)
                .distinct(field, doc! { field: { "$in": ids } })
                .max_time(QUERY_TIMEOUT)
                .await
                .map_err(query_error)?;
            found.extend(values.into_iter().filter_map(|value| match value {
                Bson::String(id) => Some(id),
                _ => None,
            }));
        }
    }
    Ok(found)
}

async fn label_ids(
    db: &mongodb::Database,
    ids: &str,
    scope: IdentityScope,
) -> AppResult<IdentityOptionsResponse> {
    let mut ids: Vec<String> = ids
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect();
    if ids.len() > MAX_IDS {
        return Err(AppError::BadRequest(format!(
            "at most {MAX_IDS} ids may be labelled at once"
        )));
    }
    if scope == IdentityScope::Usage && !ids.is_empty() {
        ids = ids_with_usage(db, &ids).await?.into_iter().collect();
    }
    let identities = reporting_identity_service::resolve(db, &ids, QUERY_TIMEOUT)
        .await
        .map_err(query_error)?;
    let options: Vec<IdentityOption> = identities
        .into_iter()
        .map(|(id, identity)| option(id, &identity))
        .collect();
    Ok(IdentityOptionsResponse {
        total: options.len() as u64,
        options,
    })
}

/// Same label as the usage report; detail distinguishes the identity type.
fn option(id: String, identity: &ReportingIdentity) -> IdentityOption {
    let detail = match identity.user_type.as_str() {
        "org" => Some("Organization".to_string()),
        "service_account" => Some("Service account".to_string()),
        _ => identity.email.clone(),
    };
    IdentityOption {
        label: identity.label().to_string(),
        id,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::{User, UserType};
    use crate::test_utils::{connect_test_database, test_user};

    async fn insert_person(db: &mongodb::Database, email: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let mut user = test_user(&id, UserType::Person);
        user.email = email.to_string();
        user.display_name = Some("   ".to_string());
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .unwrap();
        id
    }

    #[tokio::test]
    async fn usage_scope_discloses_only_identities_with_usage() {
        let Some(db) = connect_test_database("usage_identities_scope").await else {
            eprintln!("skipping usage identities test: no local MongoDB available");
            return;
        };
        let active = insert_person(&db, "active@example.test").await;
        let idle = insert_person(&db, "idle@example.test").await;
        let fresh = insert_person(&db, "fresh@example.test").await;
        db.collection::<Document>(crate::models::usage_rollup_daily::COLLECTION_NAME)
            .insert_one(doc! { "_id": "rollup-1", "actor": &active, "owner": &active })
            .await
            .unwrap();
        db.collection::<Document>(crate::models::usage_rollup_hourly::COLLECTION_NAME)
            .insert_one(doc! { "_id": "rollup-2", "actor": &fresh, "owner": &fresh })
            .await
            .unwrap();
        // Raw meter rows (including failed ones the report excludes) never
        // count; identities appear once their eligible usage is rolled up.
        db.collection::<Document>(crate::models::usage_meter::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": "meter-1",
                "actor_user_id": &idle,
                "billing_owner_id": &idle,
                "status": "failed",
            })
            .await
            .unwrap();

        let search = |scope| {
            lookup(
                &db,
                IdentityQuery {
                    kind: Some("actors".to_string()),
                    search: Some("example.test".to_string()),
                    ids: None,
                },
                scope,
            )
        };
        let usage = search(IdentityScope::Usage).await.unwrap();
        assert_eq!(usage.total, 2);
        let mut found: Vec<_> = usage
            .options
            .iter()
            .map(|option| option.id.as_str())
            .collect();
        found.sort_unstable();
        let mut expected = vec![active.as_str(), fresh.as_str()];
        expected.sort_unstable();
        assert_eq!(found, expected);
        // Whitespace display names fall back to the email, as in the report.
        assert!(
            usage
                .options
                .iter()
                .any(|option| option.label == "active@example.test")
        );
        let directory = search(IdentityScope::Directory).await.unwrap();
        assert_eq!(directory.total, 3);

        let labels = lookup(
            &db,
            IdentityQuery {
                ids: Some(format!("{active},{idle},does-not-exist")),
                ..IdentityQuery::default()
            },
            IdentityScope::Usage,
        )
        .await
        .unwrap();
        assert_eq!(
            labels
                .options
                .iter()
                .map(|option| option.id.as_str())
                .collect::<Vec<_>>(),
            vec![active.as_str()]
        );

        let too_many = vec!["id"; MAX_IDS + 1].join(",");
        assert!(
            lookup(
                &db,
                IdentityQuery {
                    ids: Some(too_many),
                    ..IdentityQuery::default()
                },
                IdentityScope::Directory,
            )
            .await
            .is_err()
        );
        db.drop().await.unwrap();
    }
}

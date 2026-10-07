use std::collections::HashMap;
use std::time::Duration;

use futures::TryStreamExt;
use mongodb::bson::{Document, doc};

#[derive(Clone)]
pub struct ReportingIdentity {
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub user_type: String,
}

impl ReportingIdentity {
    pub fn label(&self) -> &str {
        self.display_name
            .as_deref()
            .or(self.email.as_deref())
            .unwrap_or(if self.user_type == "service_account" {
                "Unnamed service account"
            } else {
                "Unnamed user"
            })
    }
}

fn nonempty_field(row: &Document, field: &str) -> Option<String> {
    row.get_str(field)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
}

/// Resolves report subjects without loading credentials or requiring an active
/// account. Historical usage and audit rows retain deactivated identities.
pub async fn resolve(
    db: &mongodb::Database,
    ids: &[String],
    max_time: Duration,
) -> mongodb::error::Result<HashMap<String, ReportingIdentity>> {
    let mut identities = HashMap::new();
    if ids.is_empty() {
        return Ok(identities);
    }
    let users: Vec<Document> = db
        .collection(crate::models::user::COLLECTION_NAME)
        .find(doc! { "_id": { "$in": ids } })
        .projection(doc! { "display_name": 1, "email": 1, "user_type": 1 })
        .max_time(max_time)
        .await?
        .try_collect()
        .await?;
    for user in users {
        if let Ok(id) = user.get_str("_id") {
            identities.insert(
                id.to_owned(),
                ReportingIdentity {
                    display_name: nonempty_field(&user, "display_name"),
                    email: nonempty_field(&user, "email"),
                    user_type: user.get_str("user_type").unwrap_or("person").into(),
                },
            );
        }
    }
    let missing: Vec<_> = ids
        .iter()
        .filter(|id| !identities.contains_key(*id))
        .collect();
    if missing.is_empty() {
        return Ok(identities);
    }
    let accounts: Vec<Document> = db
        .collection(crate::models::service_account::COLLECTION_NAME)
        .find(doc! { "_id": { "$in": missing } })
        .projection(doc! { "name": 1 })
        .max_time(max_time)
        .await?
        .try_collect()
        .await?;
    for account in accounts {
        if let Ok(id) = account.get_str("_id") {
            identities.insert(
                id.to_owned(),
                ReportingIdentity {
                    display_name: nonempty_field(&account, "name"),
                    email: None,
                    user_type: "service_account".into(),
                },
            );
        }
    }
    Ok(identities)
}

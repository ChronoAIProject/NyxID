use super::{Candidate, MAX_APPROVERS};
use crate::{
    errors::AppResult,
    models::{
        org_membership::{COLLECTION_NAME as MEMBERS, OrgRole},
        user::COLLECTION_NAME as USERS,
    },
};
use bson::{Document, doc};
use mongodb::{ClientSession, Database};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Serialize)]
pub(super) struct UnavailableApprover {
    user_id: String,
    display_name: Option<String>,
    reason: Reason,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Reason {
    NotAdmin,
    NotMember,
    OutOfScope,
    Inactive,
    Unknown,
}

#[derive(Deserialize)]
struct Membership {
    member_user_id: String,
    role: OrgRole,
    revoked_at: Option<bson::DateTime>,
    person: Option<Person>,
}

#[derive(Deserialize)]
struct Person {
    display_name: Option<String>,
    is_active: bool,
    #[serde(default)]
    platform_personal_deleting: bool,
}

pub(super) async fn for_saved_list(
    db: &Database,
    tx: &mut ClientSession,
    org: &str,
    saved: &[String],
    eligible: &[Candidate],
) -> AppResult<Vec<UnavailableApprover>> {
    if saved.is_empty() {
        return Ok(Vec::new());
    }
    let candidates: HashSet<_> = eligible.iter().map(|person| &person.user_id).collect();
    let mut missing: BTreeMap<_, _> = saved
        .iter()
        .filter(|id| !candidates.contains(id))
        .take(MAX_APPROVERS)
        .map(|id| {
            (
                id.as_str(),
                UnavailableApprover {
                    user_id: id.clone(),
                    display_name: None,
                    reason: Reason::NotMember,
                },
            )
        })
        .collect();
    if missing.is_empty() {
        return Ok(Vec::new());
    }

    // Only a membership in this org, including a revoked one, opens the person lookup.
    let pipeline = [
        doc! {"$match":{"org_user_id":org,"member_user_id":{"$in":missing.keys().copied().collect::<Vec<_>>()}}},
        doc! {"$lookup":{"from":USERS,"localField":"member_user_id","foreignField":"_id","as":"person",
        "pipeline":[{"$match":{"user_type":{"$in":[null,"person"]}}},
                    {"$project":{"_id":0,"display_name":1,"is_active":1,"platform_personal_deleting":1}}]}},
        doc! {"$project":{"_id":0,"member_user_id":1,"role":1,"revoked_at":1,"person":{"$first":"$person"}}},
    ];
    let mut memberships = db
        .collection::<Document>(MEMBERS)
        .aggregate(pipeline)
        .with_type::<Membership>()
        .session(&mut *tx)
        .await?;
    while let Some(member) = memberships.next(&mut *tx).await.transpose()? {
        if let Some(entry) = missing.get_mut(member.member_user_id.as_str()) {
            entry.reason = match member.person {
                None => Reason::Unknown,
                Some(person) => {
                    entry.display_name = person.display_name;
                    if member.revoked_at.is_some() {
                        Reason::NotMember
                    } else if !person.is_active || person.platform_personal_deleting {
                        Reason::Inactive
                    } else if member.role != OrgRole::Admin {
                        Reason::NotAdmin
                    } else {
                        // A live Admin absent from the authoritative candidate set lacks service scope.
                        Reason::OutOfScope
                    }
                }
            };
        }
    }
    Ok(missing.into_values().collect())
}

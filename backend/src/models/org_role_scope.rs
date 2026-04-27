use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::org_membership::OrgRole;

pub const COLLECTION_NAME: &str = "org_role_scopes";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgRoleScope {
    #[serde(rename = "_id")]
    pub id: String,
    /// User where `user_type = Org`.
    pub org_user_id: String,
    pub role: OrgRole,
    /// None means full access for this role.
    #[serde(default)]
    pub allowed_service_ids: Option<Vec<String>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    pub updated_by: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "org_role_scopes");
    }

    #[test]
    fn bson_roundtrip() {
        let scope = OrgRoleScope {
            id: uuid::Uuid::new_v4().to_string(),
            org_user_id: uuid::Uuid::new_v4().to_string(),
            role: OrgRole::Member,
            allowed_service_ids: Some(vec!["svc-1".to_string()]),
            updated_at: Utc::now(),
            updated_by: uuid::Uuid::new_v4().to_string(),
        };

        let doc = bson::to_document(&scope).expect("serialize");
        let restored: OrgRoleScope = bson::from_document(doc).expect("deserialize");

        assert_eq!(scope.id, restored.id);
        assert_eq!(scope.org_user_id, restored.org_user_id);
        assert_eq!(scope.role, restored.role);
        assert_eq!(scope.allowed_service_ids, restored.allowed_service_ids);
        assert_eq!(scope.updated_by, restored.updated_by);
    }
}

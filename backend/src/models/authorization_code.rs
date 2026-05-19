use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::errors::{AppError, AppResult};

pub const COLLECTION_NAME: &str = "authorization_codes";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalSubjectRef {
    pub platform: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
    pub external_user_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationCode {
    #[serde(rename = "_id")]
    pub id: String,
    pub code_hash: String,
    pub client_id: String,
    pub user_id: String,
    pub redirect_uri: String,
    pub scope: String,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub nonce: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_subject: Option<ExternalSubjectRef>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    pub used: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

pub fn validate_external_subject_params(
    platform: Option<&str>,
    tenant: Option<&str>,
    external_user_id: Option<&str>,
) -> AppResult<Option<ExternalSubjectRef>> {
    let platform = platform.map(str::trim).unwrap_or("");
    let tenant = tenant.map(str::trim).unwrap_or("");
    let external_user_id = external_user_id.map(str::trim).unwrap_or("");

    if platform.is_empty() && tenant.is_empty() && external_user_id.is_empty() {
        return Ok(None);
    }

    if !tenant.is_empty() && (platform.is_empty() || external_user_id.is_empty()) {
        return Err(AppError::ValidationError(
            "external_subject_tenant requires external_subject_platform and external_subject_external_user_id".to_string(),
        ));
    }

    if platform.is_empty() {
        return Err(AppError::ValidationError(
            "external_subject_platform is required when external subject parameters are provided"
                .to_string(),
        ));
    }

    if external_user_id.is_empty() {
        return Err(AppError::ValidationError(
            "external_subject_external_user_id is required when external subject parameters are provided"
                .to_string(),
        ));
    }

    validate_external_subject_value("external_subject_platform", platform)?;
    if !tenant.is_empty() {
        validate_external_subject_value("external_subject_tenant", tenant)?;
    }
    validate_external_subject_value("external_subject_external_user_id", external_user_id)?;

    Ok(Some(ExternalSubjectRef {
        platform: platform.to_string(),
        tenant: (!tenant.is_empty()).then(|| tenant.to_string()),
        external_user_id: external_user_id.to_string(),
    }))
}

fn validate_external_subject_value(field: &str, value: &str) -> AppResult<()> {
    if value.chars().count() > 256 {
        return Err(AppError::ValidationError(format!(
            "{field} must be at most 256 characters"
        )));
    }

    if !value.chars().all(|c| c.is_ascii() && !c.is_ascii_control()) {
        return Err(AppError::ValidationError(format!(
            "{field} must contain only printable ASCII characters"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "authorization_codes");
    }

    #[test]
    fn bson_roundtrip() {
        let code = AuthorizationCode {
            id: uuid::Uuid::new_v4().to_string(),
            code_hash: "hash123".to_string(),
            client_id: "default-client".to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            redirect_uri: "http://localhost:3000/callback".to_string(),
            scope: "openid profile".to_string(),
            code_challenge: Some("challenge".to_string()),
            code_challenge_method: Some("S256".to_string()),
            nonce: Some("nonce123".to_string()),
            external_subject: Some(ExternalSubjectRef {
                platform: "lark".to_string(),
                tenant: Some("t1".to_string()),
                external_user_id: "u1".to_string(),
            }),
            expires_at: Utc::now(),
            used: false,
            created_at: Utc::now(),
        };
        let doc = bson::to_document(&code).expect("serialize");
        let restored: AuthorizationCode = bson::from_document(doc).expect("deserialize");
        assert_eq!(code.id, restored.id);
        assert_eq!(code.scope, restored.scope);
        assert_eq!(code.code_challenge, restored.code_challenge);
        assert_eq!(code.external_subject, restored.external_subject);
    }

    #[test]
    fn bson_roundtrip_no_pkce() {
        let code = AuthorizationCode {
            id: uuid::Uuid::new_v4().to_string(),
            code_hash: "hash123".to_string(),
            client_id: "default-client".to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            redirect_uri: "http://localhost:3000/callback".to_string(),
            scope: "openid".to_string(),
            code_challenge: None,
            code_challenge_method: None,
            nonce: None,
            external_subject: None,
            expires_at: Utc::now(),
            used: true,
            created_at: Utc::now(),
        };
        let doc = bson::to_document(&code).expect("serialize");
        let restored: AuthorizationCode = bson::from_document(doc).expect("deserialize");
        assert!(restored.code_challenge.is_none());
        assert!(restored.nonce.is_none());
        assert!(restored.used);
    }

    #[test]
    fn bson_roundtrip_no_external_subject() {
        let now = Utc::now();
        let doc = bson::doc! {
            "_id": uuid::Uuid::new_v4().to_string(),
            "code_hash": "hash123",
            "client_id": "default-client",
            "user_id": uuid::Uuid::new_v4().to_string(),
            "redirect_uri": "http://localhost:3000/callback",
            "scope": "openid",
            "code_challenge": "challenge",
            "code_challenge_method": "S256",
            "nonce": "nonce123",
            "expires_at": bson::DateTime::from_chrono(now),
            "used": false,
            "created_at": bson::DateTime::from_chrono(now),
        };

        let restored: AuthorizationCode = bson::from_document(doc).expect("deserialize");
        assert!(restored.external_subject.is_none());
    }

    #[test]
    fn validate_external_subject_params_all_none() {
        let result = validate_external_subject_params(None, None, None).expect("valid");
        assert_eq!(result, None);
    }

    #[test]
    fn validate_external_subject_params_platform_and_user() {
        let result =
            validate_external_subject_params(Some("lark"), None, Some("u1")).expect("valid");
        assert_eq!(
            result,
            Some(ExternalSubjectRef {
                platform: "lark".to_string(),
                tenant: None,
                external_user_id: "u1".to_string(),
            })
        );
    }

    #[test]
    fn validate_external_subject_params_all_three() {
        let result =
            validate_external_subject_params(Some("lark"), Some("t1"), Some("u1")).expect("valid");
        assert_eq!(
            result,
            Some(ExternalSubjectRef {
                platform: "lark".to_string(),
                tenant: Some("t1".to_string()),
                external_user_id: "u1".to_string(),
            })
        );
    }

    #[test]
    fn validate_external_subject_params_rejects_tenant_only() {
        let err = validate_external_subject_params(None, Some("t1"), None).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn validate_external_subject_params_rejects_missing_user_id() {
        let err = validate_external_subject_params(Some("lark"), None, None).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn validate_external_subject_params_rejects_empty_platform() {
        let err = validate_external_subject_params(Some("  "), None, Some("u1")).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn validate_external_subject_params_rejects_oversize() {
        let oversize = "a".repeat(257);
        let err = validate_external_subject_params(Some(&oversize), None, Some("u1")).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn validate_external_subject_params_rejects_control_char() {
        let err = validate_external_subject_params(Some("la\nrk"), None, Some("u1")).unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[test]
    fn validate_external_subject_params_trims_whitespace() {
        let result =
            validate_external_subject_params(Some("  lark  "), Some("  t1  "), Some("  u1  "))
                .expect("valid");

        assert_eq!(
            result,
            Some(ExternalSubjectRef {
                platform: "lark".to_string(),
                tenant: Some("t1".to_string()),
                external_user_id: "u1".to_string(),
            })
        );
    }
}

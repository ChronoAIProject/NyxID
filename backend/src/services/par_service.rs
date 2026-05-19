use chrono::{Duration, Utc};
use mongodb::bson::{self, doc};

use crate::errors::{AppError, AppResult};
use crate::models::authorization_code::ExternalSubjectRef;
use crate::models::pushed_authorization_request::{
    COLLECTION_NAME as PAR_COLLECTION, PushedAuthorizationRequest, generate_request_uri,
    hash_request_uri,
};

/// PAR record TTL in seconds. RFC 9126 §2.2 says the lifetime "MUST be short
/// and SHOULD NOT exceed sixty seconds" — we hold the ceiling to give MFA-
/// inclusive logins room without violating the spec.
pub const PAR_TTL_SECS: i64 = 60;

#[allow(clippy::too_many_arguments)]
pub async fn create_request(
    db: &mongodb::Database,
    client_id: &str,
    response_type: &str,
    redirect_uri: &str,
    scope: Option<&str>,
    state: Option<&str>,
    code_challenge: Option<&str>,
    code_challenge_method: Option<&str>,
    nonce: Option<&str>,
    prompt: Option<&str>,
    external_subject: Option<ExternalSubjectRef>,
) -> AppResult<(String, i64)> {
    let request_uri = generate_request_uri();
    let id = hash_request_uri(&request_uri);
    let now = Utc::now();
    let expires_at = now + Duration::seconds(PAR_TTL_SECS);

    let record = PushedAuthorizationRequest {
        id,
        client_id: client_id.to_string(),
        response_type: response_type.to_string(),
        redirect_uri: redirect_uri.to_string(),
        scope: scope.map(String::from),
        state: state.map(String::from),
        code_challenge: code_challenge.map(String::from),
        code_challenge_method: code_challenge_method.map(String::from),
        nonce: nonce.map(String::from),
        prompt: prompt.map(String::from),
        external_subject,
        expires_at,
        created_at: now,
    };
    db.collection::<PushedAuthorizationRequest>(PAR_COLLECTION)
        .insert_one(&record)
        .await?;

    Ok((request_uri, PAR_TTL_SECS))
}

/// Atomically consume a request_uri. Returns the persisted params.
/// Single-use: the row is deleted on success.
pub async fn consume_request(
    db: &mongodb::Database,
    request_uri: &str,
    expected_client_id: &str,
) -> AppResult<PushedAuthorizationRequest> {
    let id = hash_request_uri(request_uri);
    let now = Utc::now();
    let record = db
        .collection::<PushedAuthorizationRequest>(PAR_COLLECTION)
        .find_one_and_delete(doc! {
            "_id": &id,
            "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
            "client_id": expected_client_id,
        })
        .await?
        .ok_or_else(|| AppError::BadRequest("invalid or expired request_uri".to_string()))?;
    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::connect_test_database;

    async fn insert_expired_request(db: &mongodb::Database, request_uri: &str, client_id: &str) {
        let now = Utc::now();
        let record = PushedAuthorizationRequest {
            id: hash_request_uri(request_uri),
            client_id: client_id.to_string(),
            response_type: "code".to_string(),
            redirect_uri: "https://client.example/callback".to_string(),
            scope: Some("openid".to_string()),
            state: None,
            code_challenge: Some("challenge".to_string()),
            code_challenge_method: Some("S256".to_string()),
            nonce: None,
            prompt: None,
            external_subject: None,
            expires_at: now - Duration::seconds(1),
            created_at: now - Duration::seconds(PAR_TTL_SECS + 1),
        };
        db.collection::<PushedAuthorizationRequest>(PAR_COLLECTION)
            .insert_one(&record)
            .await
            .expect("insert expired PAR record");
    }

    #[tokio::test]
    async fn create_request_returns_uri_with_prefix_and_length() {
        let Some(db) = connect_test_database("par_create").await else {
            return;
        };

        let (request_uri, expires_in) = create_request(
            &db,
            "client-par",
            "code",
            "https://client.example/callback",
            Some("openid profile"),
            Some("state-1"),
            Some("challenge"),
            Some("S256"),
            Some("nonce-1"),
            None,
            None,
        )
        .await
        .expect("create PAR");

        assert_eq!(expires_in, PAR_TTL_SECS);
        assert!(
            request_uri
                .starts_with(crate::models::pushed_authorization_request::REQUEST_URI_PREFIX)
        );
        let suffix =
            &request_uri[crate::models::pushed_authorization_request::REQUEST_URI_PREFIX.len()..];
        assert_eq!(
            suffix.len(),
            crate::models::pushed_authorization_request::REQUEST_URI_RANDOM_HEX_LEN
        );
    }

    #[tokio::test]
    async fn consume_request_returns_created_params() {
        let Some(db) = connect_test_database("par_consume").await else {
            return;
        };
        let external_subject = ExternalSubjectRef {
            platform: "lark".to_string(),
            tenant: Some("tenant-1".to_string()),
            external_user_id: "user-x".to_string(),
        };

        let (request_uri, _) = create_request(
            &db,
            "client-par",
            "code",
            "https://client.example/callback",
            Some("openid profile"),
            Some("state-1"),
            Some("challenge"),
            Some("S256"),
            Some("nonce-1"),
            Some("consent"),
            Some(external_subject.clone()),
        )
        .await
        .expect("create PAR");

        let record = consume_request(&db, &request_uri, "client-par")
            .await
            .expect("consume PAR");
        assert_eq!(record.client_id, "client-par");
        assert_eq!(record.response_type, "code");
        assert_eq!(record.redirect_uri, "https://client.example/callback");
        assert_eq!(record.scope.as_deref(), Some("openid profile"));
        assert_eq!(record.state.as_deref(), Some("state-1"));
        assert_eq!(record.code_challenge.as_deref(), Some("challenge"));
        assert_eq!(record.code_challenge_method.as_deref(), Some("S256"));
        assert_eq!(record.nonce.as_deref(), Some("nonce-1"));
        assert_eq!(record.prompt.as_deref(), Some("consent"));
        assert_eq!(record.external_subject, Some(external_subject));
    }

    #[tokio::test]
    async fn consume_request_rejects_expired_record() {
        let Some(db) = connect_test_database("par_expired").await else {
            return;
        };
        let request_uri = generate_request_uri();
        insert_expired_request(&db, &request_uri, "client-par").await;

        let result = consume_request(&db, &request_uri, "client-par").await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn consume_request_rejects_wrong_client_id() {
        let Some(db) = connect_test_database("par_wrong_client").await else {
            return;
        };
        let (request_uri, _) = create_request(
            &db,
            "client-a",
            "code",
            "https://client.example/callback",
            Some("openid"),
            None,
            Some("challenge"),
            Some("S256"),
            None,
            None,
            None,
        )
        .await
        .expect("create PAR");

        let result = consume_request(&db, &request_uri, "client-b").await;
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn consume_request_is_single_use() {
        let Some(db) = connect_test_database("par_single_use").await else {
            return;
        };
        let (request_uri, _) = create_request(
            &db,
            "client-par",
            "code",
            "https://client.example/callback",
            Some("openid"),
            None,
            Some("challenge"),
            Some("S256"),
            None,
            None,
            None,
        )
        .await
        .expect("create PAR");

        consume_request(&db, &request_uri, "client-par")
            .await
            .expect("first consume");
        let second = consume_request(&db, &request_uri, "client-par").await;
        assert!(matches!(second, Err(AppError::BadRequest(_))));
    }
}

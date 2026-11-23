use super::*;
use crate::models::catalog_delegation_grant::{COLLECTION_NAME as GRANTS, CatalogDelegationGrant};
use crate::services::{catalog_delegation_service as catalog, token_exchange_service as exchange};

tokio::task_local! { static DELAY_ISSUANCE_READ: (); }

pub(in crate::services::oauth_token_authority_service) async fn with_test_delay<T>(
    read: impl Future<Output = T>,
) -> T {
    if DELAY_ISSUANCE_READ.try_with(|_| ()).is_ok() {
        READ_FAULT
            .scope(
                ReadFault {
                    collection: crate::models::oauth_client::COLLECTION_NAME,
                    kind: ReadFaultKind::Timeout,
                },
                read,
            )
            .await
    } else {
        read.await
    }
}

async fn issue(fixture: &Fixture) -> AppResult<exchange::TokenExchangeResponse> {
    let consent = consent_service::grant_consent_with_services(
        &fixture.state.db,
        &fixture.user.to_string(),
        &fixture.client,
        "openid",
        Some(vec![]),
    )
    .await
    .unwrap();
    let reference = consent_service::ensure_issuance_reference(&fixture.state.db, &consent)
        .await
        .unwrap();
    let source = jwt::generate_oauth_access_token_with_consent(
        &fixture.state.jwt_keys,
        &fixture.state.config,
        &fixture.user,
        "openid",
        None,
        None,
        None,
        None,
        Some(jwt::AccessTokenRestrictions {
            resources: &[],
            allowed_service_ids: &[],
            allowed_node_ids: &[],
            allow_all_nodes: false,
        }),
        &fixture.client,
        Some(&reference),
    )
    .unwrap();
    exchange::exchange_token_with_authority(
        &fixture.state.db,
        &fixture.state.config,
        &fixture.state.jwt_keys,
        &fixture.client,
        "fixture-client-secret",
        &source,
        "urn:ietf:params:oauth:token-type:access_token",
        Some(catalog::MCP_CATALOG_READ_SCOPE),
        &[],
        Some(false),
        &[],
        Some(false),
        &[],
    )
    .await
}

async fn row_counts(db: &Database) -> (u64, u64) {
    let grants = db
        .collection::<CatalogDelegationGrant>(GRANTS)
        .count_documents(doc! {})
        .await
        .unwrap();
    let authorities = db
        .collection::<OauthDelegationAuthority>(COLLECTION_NAME)
        .count_documents(doc! {})
        .await
        .unwrap();
    (grants, authorities)
}

#[tokio::test]
async fn late_authority_read_refusal_leaves_no_issuance_rows() {
    let fixture = Fixture::new().await;
    let result = DELAY_ISSUANCE_READ.scope((), issue(&fixture)).await;
    assert!(matches!(result, Err(AppError::OAuthAuthorityUnavailable)));
    assert_eq!(
        row_counts(&fixture.state.db).await,
        (0, 0),
        "(catalog grants, delegated authorities)"
    );
}

#[tokio::test]
async fn catalog_exchange_and_refresh_persist_both_rows_for_each_signed_token() {
    let fixture = Fixture::new().await;
    let db = &fixture.state.db;
    let issued = issue(&fixture).await.unwrap();
    let claims = fixture.claims(&issued.access_token);
    let refreshed = exchange::refresh_delegation_token(
        db,
        &fixture.state.config,
        &fixture.state.jwt_keys,
        &claims.sub,
        &fixture.client,
        Some(&fixture.client),
        &claims.scope,
        &TokenRestrictionClaims::from_claims(&claims),
        &claims.jti,
    )
    .await
    .unwrap();
    let successor = fixture.claims(&refreshed.access_token);
    assert_ne!(claims.jti, successor.jti);
    assert_eq!(row_counts(db).await, (2, 2));
    for signed in [&claims, &successor] {
        catalog::validate_live_grant(db, &fixture.state.config, signed)
            .await
            .unwrap();
        validate_claims(db, signed).await.unwrap();
        let grant = db
            .collection::<CatalogDelegationGrant>(GRANTS)
            .find_one(doc! {"_id": &signed.jti})
            .await
            .unwrap()
            .unwrap();
        let authority = load_delegation(db, &signed.jti).await.unwrap();
        assert_eq!(grant.user_id, authority.user_id);
        assert_eq!(grant.actor_client_id, authority.actor_client_id);
        assert_eq!(
            Some(&grant.receiving_client_id),
            authority.receiving_client_id.as_ref()
        );
        assert_eq!(grant.scope, authority.scope);
        assert_eq!(grant.expires_at, authority.expires_at);
        assert_eq!(grant.expires_at.timestamp(), signed.exp);
    }
}

#[tokio::test]
async fn rejected_authority_insert_rolls_back_catalog_issuance() {
    let fixture = Fixture::new().await;
    let db = &fixture.state.db;
    db.create_collection(COLLECTION_NAME)
        .validator(doc! {"receiving_client_id": {"$ne": &fixture.client}})
        .await
        .unwrap();
    let result = issue(&fixture).await;
    let Err(AppError::DatabaseError(error)) = result else {
        panic!("expected authority row validation to reject the second insert");
    };
    use mongodb::error::{ErrorKind, WriteFailure};
    assert!(
        matches!(error.kind.as_ref(), ErrorKind::Write(WriteFailure::WriteError(e)) if e.code == 121)
            || matches!(error.kind.as_ref(), ErrorKind::Command(e) if e.code == 121)
    );
    assert_eq!(
        row_counts(db).await,
        (0, 0),
        "(catalog grants, delegated authorities)"
    );
}

#[tokio::test]
async fn accepted_owner_erasure_refuses_catalog_refresh_without_writing_a_successor() {
    let fixture = Fixture::new().await;
    let db = &fixture.state.db;
    let issued = issue(&fixture).await.unwrap();
    let claims = fixture.claims(&issued.access_token);
    let mut tx = db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    crate::services::erasure_admission::close_in_session(db, &claims.sub, &mut tx)
        .await
        .unwrap();
    tx.commit_transaction().await.unwrap();

    let result = exchange::refresh_delegation_token(
        db,
        &fixture.state.config,
        &fixture.state.jwt_keys,
        &claims.sub,
        &fixture.client,
        Some(&fixture.client),
        &claims.scope,
        &TokenRestrictionClaims::from_claims(&claims),
        &claims.jti,
    )
    .await;
    assert!(matches!(result, Err(AppError::Conflict(reason)) if reason.contains("erasure")));
    assert_eq!(row_counts(db).await, (1, 1));
    assert!(
        db.collection::<CatalogDelegationGrant>(GRANTS)
            .find_one(doc! {"_id": &claims.jti})
            .await
            .unwrap()
            .is_some()
    );
    load_delegation(db, &claims.jti).await.unwrap();
}

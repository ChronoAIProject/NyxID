use super::*;
use crate::test_utils::{cached_test_jwt_keys, test_app_config};

fn configs() -> (AppConfig, AppConfig) {
    let mut old = test_app_config();
    old.base_url = "https://old.example".into();
    old.jwt_issuer = old.base_url.clone();
    let mut new = old.clone();
    new.base_url = "https://new.example".into();
    new.jwt_issuer = new.base_url.clone();
    new.domain_aliases.base_urls = vec![old.base_url.clone()];
    (old, new)
}

#[test]
fn domain_alias_tokens_survive_cutover_without_widening_restrictions() {
    let keys = cached_test_jwt_keys();
    let (old, new) = configs();
    let user = Uuid::new_v4();
    let restrictions = TokenRestrictionClaims {
        resources: Some(vec![format!("{}/api/v1/proxy/s/test", old.base_url)]),
        allowed_service_ids: Some(vec![Uuid::new_v4().to_string()]),
        allowed_node_ids: Some(Vec::new()),
        allow_all_services: Some(false),
        allow_all_nodes: Some(false),
        ..Default::default()
    };
    let relay_scope = RelayAgentScope {
        api_key_id: Uuid::new_v4().to_string(),
        api_key_name: "agent".into(),
        allowed_service_ids: restrictions.allowed_service_ids.clone().unwrap(),
        allowed_node_ids: Vec::new(),
        allow_all_services: false,
        allow_all_nodes: false,
    };
    let tokens = [
        generate_access_token(
            &keys,
            &old,
            &user,
            "openid proxy",
            None,
            None,
            None,
            None,
            Some(AccessTokenRestrictions {
                resources: restrictions.resources.as_ref().unwrap(),
                allowed_service_ids: restrictions.allowed_service_ids.as_ref().unwrap(),
                allowed_node_ids: &[],
                allow_all_nodes: false,
            }),
        )
        .unwrap(),
        generate_refresh_token(&keys, &old, &user).unwrap().0,
        generate_service_account_token(&keys, &old, &user.to_string(), "proxy", 300, 1)
            .unwrap()
            .0,
        generate_delegated_access_token(
            &keys,
            &old,
            &user,
            "proxy",
            "test-service",
            300,
            Some(&restrictions),
        )
        .unwrap(),
        generate_relay_access_token(&keys, &old, &user, "proxy", None, &relay_scope).unwrap(),
    ];
    let mut without_alias = new.clone();
    without_alias.domain_aliases = Default::default();
    for token in &tokens {
        let before = verify_token(&keys, &old, token).unwrap();
        let after = verify_token(&keys, &new, token).unwrap();
        assert_eq!(
            serde_json::to_value(&before).unwrap(),
            serde_json::to_value(&after).unwrap()
        );
        assert!(verify_token(&keys, &without_alias, token).is_err());
    }
    let token = generate_delegated_access_token(
        &keys,
        &new,
        &user,
        "proxy",
        "test-service",
        300,
        Some(&restrictions),
    )
    .unwrap();
    let claims = verify_token(&keys, &new, &token).unwrap();
    assert_eq!(
        claims.resources,
        Some(vec!["https://new.example/api/v1/proxy/s/test".into()])
    );
    assert_eq!(claims.allowed_service_ids, restrictions.allowed_service_ids);
    assert_eq!(claims.allow_all_nodes, Some(false));
}

#[test]
fn domain_alias_relay_special_audiences_remain_strict() {
    let keys = cached_test_jwt_keys();
    let (old, new) = configs();
    let id = Uuid::new_v4().to_string();
    let reply = generate_relay_reply_token(&keys, &old, &id, &id, &id, "lark").unwrap();
    let callback = generate_relay_callback_token(
        &keys,
        &old,
        &id,
        &id,
        &id,
        "lark",
        &hex::encode(Sha256::digest([])),
    )
    .unwrap();
    assert!(validate_relay_reply_token(&keys, &new, &reply).is_ok());
    assert!(validate_relay_callback_token(&keys, &new, &callback).is_ok());
    assert!(verify_token(&keys, &new, &reply).is_err());
    assert!(verify_token(&keys, &new, &callback).is_err());
    assert!(validate_relay_reply_token(&keys, &new, &callback).is_err());
}

#[test]
fn domain_alias_pinned_minting_and_wrong_audience() {
    let keys = cached_test_jwt_keys();
    let (old, mut new) = configs();
    new.jwt_issuer = old.jwt_issuer.clone();
    new.domain_aliases.jwt_audience = Some(old.base_url.clone());
    let user = Uuid::new_v4();
    let token =
        generate_access_token(&keys, &new, &user, "openid", None, None, None, None, None).unwrap();
    let mut claims = verify_token(&keys, &old, &token).unwrap();
    assert_eq!(claims.iss, old.jwt_issuer);
    assert_eq!(claims.aud, old.base_url);
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(keys.kid.clone());
    claims.aud = "https://unconfigured.example".into();
    let invalid = encode(&header, &claims, &keys.encoding).unwrap();
    assert!(verify_token(&keys, &new, &invalid).is_err());
    claims.aud = old.base_url;
    claims.iss = "https://unconfigured.example".into();
    let invalid = encode(&header, &claims, &keys.encoding).unwrap();
    assert!(verify_token(&keys, &new, &invalid).is_err());
}

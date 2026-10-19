use super::*;
use crate::{
    AppState,
    models::{
        user::UserType,
        user_api_key::{COLLECTION_NAME as KEYS, UserApiKey},
    },
    test_utils::{connect_transaction_test_database, test_app_state, test_user},
};

const SECRET: &str = "sk-fixture-codex-file-not-a-real-key";

async fn fixture(name: &str) -> (AppState, String) {
    let db = connect_transaction_test_database(name).await;
    crate::db::ensure_indexes(&db).await.unwrap();
    let state = test_app_state(db);
    crate::services::provider_service::seed_default_providers(&state.db, &state.encryption_keys)
        .await
        .unwrap();
    crate::services::provider_service::seed_default_services(&state.db, &state.encryption_keys)
        .await
        .unwrap();
    let user = Uuid::new_v4().to_string();
    state
        .db
        .collection::<User>(USERS)
        .insert_one(test_user(&user, UserType::Person))
        .await
        .unwrap();
    (state, user)
}

#[tokio::test]
async fn import_replacement_and_verification_bind_exact_credential_epoch_and_live_service() {
    let (state, user) = fixture("codex_import_epoch").await;
    let token = import_api_key(&state.db, &state.encryption_keys, &user, SECRET, None)
        .await
        .unwrap();
    let service = registered_service(&state.db, &token).await.unwrap();
    let binding = verification_binding(&state.db, &token, &service)
        .await
        .unwrap();
    assert_eq!(binding.credential_epoch, 1);
    assert_eq!(
        state
            .encryption_keys
            .decrypt(token.api_key_encrypted.as_ref().unwrap())
            .await
            .unwrap()
            .as_slice(),
        SECRET.as_bytes()
    );
    assert_eq!(connection_status(&state.db, &token).await.unwrap(), "saved");
    record_verification(&state.db, &user, &version(&token), &binding, "usable")
        .await
        .unwrap();
    let verified = require_current(&state.db, &user, &version(&token))
        .await
        .unwrap();
    assert_eq!(
        connection_status(&state.db, &verified).await.unwrap(),
        "usable"
    );
    assert_eq!(
        verification_binding(&state.db, &token, &service)
            .await
            .unwrap(),
        binding
    );

    let replacement = import_api_key(
        &state.db,
        &state.encryption_keys,
        &user,
        SECRET,
        Some(&version(&token)),
    )
    .await
    .unwrap();
    assert_eq!(replacement.state_version, token.state_version + 1);
    assert_eq!(
        verification_binding(&state.db, &replacement, &service)
            .await
            .unwrap()
            .credential_epoch,
        2,
        "same-value user replacement must advance authority"
    );
    assert_eq!(
        connection_status(&state.db, &replacement).await.unwrap(),
        "saved"
    );
    assert!(registered_service(&state.db, &token).await.is_err());
    assert!(
        record_verification(&state.db, &user, &version(&token), &binding, "usable")
            .await
            .is_err()
    );
    let binding = verification_binding(&state.db, &replacement, &service)
        .await
        .unwrap();
    record_verification(&state.db, &user, &version(&replacement), &binding, "usable")
        .await
        .unwrap();
    let current = require_current(&state.db, &user, &version(&replacement))
        .await
        .unwrap();
    state
        .db
        .collection::<UserService>(SERVICES)
        .update_one(doc! {"_id":&service}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    assert_eq!(
        connection_status(&state.db, &current).await.unwrap(),
        "reconnect_required"
    );
    assert!(
        registered_service(&state.db, &current).await.is_err(),
        "verification cannot recreate disabled services"
    );
    state
        .db
        .collection::<UserService>(SERVICES)
        .delete_one(doc! {"_id":&service})
        .await
        .unwrap();
    assert_eq!(
        connection_status(&state.db, &current).await.unwrap(),
        "reconnect_required"
    );
    assert_eq!(
        state
            .db
            .collection::<UserApiKey>(KEYS)
            .count_documents(doc! {"user_id":&user})
            .await
            .unwrap(),
        1
    );
    state
        .db
        .collection::<UserApiKey>(KEYS)
        .update_one(
            doc! {"_id":&binding.key_id},
            doc! {"$set":{"status":"revoked"}},
        )
        .await
        .unwrap();
    let reconnected = import_api_key(
        &state.db,
        &state.encryption_keys,
        &user,
        SECRET,
        Some(&version(&current)),
    )
    .await
    .unwrap();
    let new_service = registered_service(&state.db, &reconnected).await.unwrap();
    assert_ne!(new_service, service);
    let new_binding = verification_binding(&state.db, &reconnected, &new_service)
        .await
        .unwrap();
    assert_ne!(new_binding.key_id, binding.key_id);
    assert_eq!(
        state
            .db
            .collection::<UserApiKey>(KEYS)
            .find_one(doc! {"_id":&binding.key_id})
            .await
            .unwrap()
            .unwrap()
            .status,
        "revoked"
    );
}

#[tokio::test]
async fn reconnect_after_provider_disconnect_and_service_delete_preserves_tombstones() {
    use crate::services::{audit_service::AuditActor, user_service_service};
    let (state, user) = fixture("codex_disconnect_reimport").await;
    let actor = AuditActor {
        user_id: user.clone(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    let token = import_api_key(&state.db, &state.encryption_keys, &user, SECRET, None)
        .await
        .unwrap();
    let original_service = registered_service(&state.db, &token).await.unwrap();
    user_service_service::deactivate_user_service(&state.db, &user, &user, &original_service)
        .await
        .unwrap();
    unified::disconnect_credentials(
        &state.db,
        &state.encryption_keys,
        &user,
        &actor,
        unified::DisconnectTarget::Provider(&token.provider_config_id),
        unified::DisconnectOptions::default(),
    )
    .await
    .unwrap();
    let disconnected = current(&state.db, &user, &token.provider_config_id)
        .await
        .unwrap();
    let expected = disconnected.as_ref().map(version);
    let reimported = import_api_key(
        &state.db,
        &state.encryption_keys,
        &user,
        SECRET,
        expected.as_ref(),
    )
    .await
    .unwrap();
    let service = registered_service(&state.db, &reimported).await.unwrap();
    assert_ne!(original_service, service);
    assert!(
        !state
            .db
            .collection::<UserService>(SERVICES)
            .find_one(doc! {"_id":&original_service})
            .await
            .unwrap()
            .unwrap()
            .is_active
    );
    let binding = verification_binding(&state.db, &reimported, &service)
        .await
        .unwrap();
    record_verification(&state.db, &user, &version(&reimported), &binding, "usable")
        .await
        .unwrap();
    unified::disconnect_credentials(
        &state.db,
        &state.encryption_keys,
        &user,
        &actor,
        unified::DisconnectTarget::UserService(&service),
        unified::DisconnectOptions::default(),
    )
    .await
    .unwrap();
    assert!(
        !state
            .db
            .collection::<UserService>(SERVICES)
            .find_one(doc! {"_id":&service})
            .await
            .unwrap()
            .unwrap()
            .is_active
    );
    let deleted = current(&state.db, &user, &token.provider_config_id)
        .await
        .unwrap();
    let expected = deleted.as_ref().map(version);
    let reimported = import_api_key(
        &state.db,
        &state.encryption_keys,
        &user,
        SECRET,
        expected.as_ref(),
    )
    .await
    .unwrap();
    let newest_service = registered_service(&state.db, &reimported).await.unwrap();
    assert_ne!(newest_service, service);
    assert_ne!(newest_service, original_service);
    assert_eq!(
        state
            .db
            .collection::<UserService>(SERVICES)
            .count_documents(doc! {"user_id":&user})
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        state
            .db
            .collection::<UserService>(SERVICES)
            .count_documents(doc! {"user_id":&user,"is_active":true})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        connection_status(&state.db, &reimported).await.unwrap(),
        "saved"
    );
}

#[tokio::test]
async fn concurrent_import_and_failed_provision_leave_no_partial_resources() {
    let (state, user) = fixture("codex_import_atomic").await;
    let endpoints = crate::models::user_endpoint::COLLECTION_NAME;
    state.db.run_command(doc! {"collMod":SERVICES,"validator":{"reject_fixture":true},"validationLevel":"strict","validationAction":"error"}).await.unwrap();
    assert!(
        import_api_key(&state.db, &state.encryption_keys, &user, SECRET, None)
            .await
            .is_err()
    );
    for name in [TOKENS, KEYS, SERVICES, endpoints] {
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(name)
                .count_documents(doc! {"user_id":&user})
                .await
                .unwrap(),
            0
        );
    }
    state
        .db
        .run_command(doc! {"collMod":SERVICES,"validator":{}})
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        import_api_key(&state.db, &state.encryption_keys, &user, SECRET, None),
        import_api_key(
            &state.db,
            &state.encryption_keys,
            &user,
            "sk-fixture-other",
            None
        )
    );
    assert_eq!(
        [a.is_ok(), b.is_ok()].into_iter().filter(|ok| *ok).count(),
        1
    );
    let token = a.or(b).unwrap();
    let expected = version(&token);
    let (a, b) = tokio::join!(
        import_api_key(
            &state.db,
            &state.encryption_keys,
            &user,
            SECRET,
            Some(&expected)
        ),
        import_api_key(
            &state.db,
            &state.encryption_keys,
            &user,
            SECRET,
            Some(&expected)
        )
    );
    assert_eq!(
        [a.is_ok(), b.is_ok()].into_iter().filter(|ok| *ok).count(),
        1
    );
    for name in [TOKENS, KEYS, SERVICES, endpoints] {
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(name)
                .count_documents(doc! {"user_id":&user})
                .await
                .unwrap(),
            1
        );
    }
    let wrong_provider = provider(&state.db).await.unwrap();
    state
        .db
        .collection::<ProviderConfig>(PROVIDERS)
        .update_one(
            doc! {"_id":wrong_provider.id},
            doc! {"$set":{"provider_type":"device_code"}},
        )
        .await
        .unwrap();
    assert!(
        require_current(&state.db, &user, &version(&a.or(b).unwrap()))
            .await
            .is_err()
    );
}

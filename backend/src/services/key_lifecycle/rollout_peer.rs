//! Loopback adapter compiled unchanged against each side of the mixed-binary rehearsal.
use super::*;
use crate::{
    crypto::{aes::EncryptionKeys, jwt},
    services::security_readiness_service as fleet,
    test_utils,
};
use std::sync::Arc;

#[tokio::test]
#[ignore = "child process for the opt-in mixed-release rehearsal"]
async fn serve() {
    test_utils::init_test_audit_chain_hmac_key();
    let variable = |name| std::env::var(name).unwrap();
    let url = variable("NYXID_BOOTSTRAP_DATABASE_URL");
    let db = crate::db::create_connection_with_url(&url, 8)
        .await
        .unwrap();
    assert!(db.name().starts_with("nyxid_test_") && db.name().ends_with("_rkroll"));
    let mut config = test_utils::test_app_config();
    config.encryption_key = Some(variable("ENCRYPTION_KEY"));
    config.encryption_key_previous = std::env::var("ENCRYPTION_KEY_PREVIOUS").ok();
    config.hmac_keyset_path = Some(variable("HMAC_KEYSET_PATH"));
    config.contextual_record_writes = true;
    config.allow_legacy_record_reads = false;
    let ready = std::path::PathBuf::from(variable("NYXID_REHEARSAL_READY"));
    let admission = match admit_startup(&db, &config).await {
        Ok(ticket) => ticket,
        Err(crate::errors::AppError::Conflict(_)) => {
            std::fs::write(ready.with_extension("refused"), b"ring_admission").unwrap();
            panic!("ring admission refused");
        }
        Err(_) => panic!("unexpected startup error"),
    };
    let init = retirement::Initialization::begin(&db, admission.as_ref())
        .await
        .unwrap();
    crate::db::initialize(&db).await.unwrap();
    let keys = EncryptionKeys::from_config(&config);
    let mut state = test_utils::test_app_state_with_config(db.clone(), config);
    state.encryption_keys = Arc::new(keys);
    let mut runtime = fleet::Runtime::new(
        (*state.replica_identity).clone(),
        fleet::RolloutConfig {
            expected_replicas: vec!["coordinator".into(), "traffic".into()],
            drained: true,
        },
        false,
    );
    runtime.identity.instance_name = "traffic".into();
    runtime.retirement_cohort = admission.and_then(|a| a.cohort);
    fleet::initialize(runtime.clone());
    fleet::observe_with_automation(&db, &runtime, false)
        .await
        .unwrap();
    init.finish(&db).await.unwrap();
    let owner = uuid::Uuid::parse_str(&variable("NYXID_REHEARSAL_OWNER")).unwrap();
    let token = jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &owner,
        "openid profile",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (public, internal) = crate::routes::build_router_with_state(state.clone());
    let app = public.merge(internal).with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let tmp = ready.with_extension("tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec(&json!({"port":listener.local_addr().unwrap().port(),"token":token}))
            .unwrap(),
    )
    .unwrap();
    std::fs::rename(tmp, ready).unwrap();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            fleet::observe_with_automation(&db, &runtime, false)
                .await
                .unwrap();
        }
    });
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .unwrap();
}

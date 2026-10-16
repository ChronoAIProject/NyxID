#![cfg(test)]
pub(crate) async fn funded_fixture() -> Fixture {
    super::platform_inference_service::funded_test_fixture().await
}

use super::{
    platform_provisioning_service as provisioning, platform_setup_service as setups,
    telegram_new_tests as telegram,
};
use crate::{
    AppState,
    models::{
        platform_binding::PlatformBinding,
        platform_bot_config::{GuideConfiguration, PlatformBotConfig},
        platform_installation::SetupStepJob,
        platform_setup::{PlatformSetup, SetupFacts, SetupTemplate},
    },
};
use mongodb::bson::doc;
pub(crate) struct Fixture {
    pub state: AppState,
    pub owner: String,
    pub operator: String,
    pub server: wiremock::MockServer,
    pub profile: PlatformBotConfig,
    pub binding: PlatformBinding,
    pub setup: PlatformSetup,
}

pub(crate) async fn fixture() -> Fixture {
    fixture_inner(false).await
}
pub(crate) async fn distinct_fixture() -> Fixture {
    fixture_inner(true).await
}
async fn fixture_inner(distinct: bool) -> Fixture {
    let (state, operator, server) = telegram::fixture().await;
    for result in [
        setups::ensure_indexes(&state.db).await,
        super::platform_bot_config_service::ensure_indexes(&state.db).await,
        super::platform_binding_service::ensure_indexes(&state.db).await,
        provisioning::ensure_indexes(&state.db).await,
        super::platform_inference_service::ensure_indexes(&state.db).await,
        super::platform_native_receipt_service::ensure_indexes(&state.db).await,
    ] {
        result.unwrap();
    }
    super::role_service::seed_system_roles(&state.db)
        .await
        .unwrap();
    let admin_id = super::role_service::get_platform_role_ids(&state.db)
        .await
        .unwrap()
        .admin;
    state
        .db
        .collection::<mongodb::bson::Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &operator},
            doc! {"$set": {"role_ids": [&admin_id]}},
        )
        .await
        .unwrap();
    let bot = telegram::register_manager_channel(&state, &operator, &server).await;
    let funding =
        super::org_service::create_org_user(&state.db, "Dedicated test sponsor", None, None)
            .await
            .unwrap()
            .id;
    super::org_service::create_membership(
        &state.db,
        &funding,
        &operator,
        crate::models::org_membership::OrgRole::Admin,
        crate::models::org_membership::MemberScopeSource::Inherit,
        None,
    )
    .await
    .unwrap();
    let mut profile =
        super::platform_bot_config_service::convert(&state.db, &operator, &bot.id, &funding)
            .await
            .unwrap();
    profile.enabled = true;
    profile.guide = Some(GuideConfiguration {
        model_service_id: uuid::Uuid::new_v4().to_string(),
        model_id: "test-model".into(),
        limits: setups::default_limits(),
        anonymous_daily_credits: "0.1".parse().unwrap(),
        platform_turns_per_hour: 50,
        platform_concurrent_turns: 2,
    });
    state
        .db
        .collection::<PlatformBotConfig>(crate::models::platform_bot_config::COLLECTION_NAME)
        .replace_one(doc! {"_id": &profile.id}, &profile)
        .await
        .unwrap();
    let owner = if distinct {
        let client = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &client,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        client
    } else {
        operator.clone()
    };
    let key = state.auth_device_hmac_key.as_slice();
    let (_, code) =
        super::platform_binding_service::begin(&state.db, key, &profile, "700", "700", 100)
            .await
            .unwrap();
    super::platform_binding_service::decide(&state.db, key, &owner, &code, &code, true, false)
        .await
        .unwrap();
    let binding = super::platform_binding_service::confirm_from_chat(
        &state.db, key, &profile, "700", "700", &code,
    )
    .await
    .unwrap();
    let facts = SetupFacts {
        name: "Client cafe".into(),
        timezone: "UTC".into(),
        hours: "09:00–18:00".into(),
        address: "Public address".into(),
        contact: "Public phone".into(),
        ..Default::default()
    };
    let setup = setups::create_owned(
        &state.db,
        &state.encryption_keys,
        &owner,
        &uuid::Uuid::new_v4().to_string(),
        SetupTemplate::RestaurantStarter,
        &facts,
    )
    .await
    .unwrap();
    let setup = setups::attach_binding(
        &state.db,
        &owner,
        &setup.id,
        setup.revision,
        &binding.id,
        binding.generation,
    )
    .await
    .unwrap();
    let setup = setups::publish_reviewed(
        &state.db,
        &state.encryption_keys,
        &owner,
        &setup.id,
        setup.revision,
    )
    .await
    .unwrap();
    let setup = setups::request_review(
        &state.db,
        &state.encryption_keys,
        &owner,
        &setup.id,
        setup.revision,
    )
    .await
    .unwrap();
    let digest = setups::approval_digest(&setup, &facts).unwrap();
    let setup = provisioning::approve_owned(
        &state.db,
        &state.encryption_keys,
        &owner,
        &setup.id,
        setup.revision,
        &digest,
    )
    .await
    .unwrap();
    Fixture {
        state,
        owner,
        operator,
        server,
        profile,
        binding,
        setup,
    }
}

impl Fixture {
    pub(crate) async fn claim(&self) -> SetupStepJob {
        use std::time::Duration;

        // Enqueue and eligibility use application and MongoDB clocks. Like the
        // worker, wait for a real claim instead of assuming immediate eligibility.
        let claimed = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(job) = provisioning::claim_step(&self.state.db)
                    .await
                    .expect("provisioning claim query must succeed")
                {
                    return job;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        if let Ok(job) = claimed {
            return job;
        }
        let diagnostic = tokio::time::timeout(Duration::from_secs(1), async {
            self.state
                .db
                .collection::<mongodb::bson::Document>(crate::models::platform_installation::STEPS)
                .find_one(doc! {"setup_id": &self.setup.id})
                .projection(
                    doc! {"_id": 1, "phase": 1, "generation": 1, "attempts": 1, "retry_at": 1},
                )
                .await
        })
        .await;
        panic!(
            "timed out waiting for provisioning claim; database={}, setup={}, step={diagnostic:?}",
            self.state.db.name(),
            self.setup.id,
        );
    }
    pub(crate) async fn native_start(
        &self,
    ) -> crate::models::telegram_bot_request::TelegramBotRequest {
        let job = self.claim().await;
        let base = self.server.uri();
        let service = telegram::service(&self.state, &base);
        let (request, url) = service.begin_approved(&job, "Client cafe").await.unwrap();
        let challenge = reqwest::Url::parse(&url)
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == "start")
            .unwrap()
            .1
            .to_string();
        let update = telegram::message(serde_json::json!({"text": format!("/start {challenge}")}));
        service
            .webhook(
                &telegram::headers(&self.state).await,
                &serde_json::to_vec(&update).unwrap(),
            )
            .await
            .unwrap();
        self.state
            .db
            .collection::<crate::models::telegram_bot_request::TelegramBotRequest>(
                crate::models::telegram_bot_request::COLLECTION_NAME,
            )
            .find_one(doc! {"_id": request.id})
            .await
            .unwrap()
            .unwrap()
    }
    pub(crate) async fn observe(&self) -> crate::models::telegram_bot_request::TelegramBotRequest {
        let request = self.native_start().await;
        let base = self.server.uri();
        let service = telegram::service(&self.state, &base);
        let headers = telegram::headers(&self.state).await;
        let child = serde_json::json!({"id": 900, "is_bot": true, "username": "CustomerBot"});
        service.webhook(&headers, &serde_json::to_vec(&serde_json::json!({"update_id": 2, "managed_bot": {"user": {"id": 700, "is_bot": false}, "bot": child}})).unwrap()).await.unwrap();
        let mut update =
            telegram::message(serde_json::json!({"managed_bot_created": {"bot": child}}));
        update["update_id"] = serde_json::json!(3);
        service
            .webhook(&headers, &serde_json::to_vec(&update).unwrap())
            .await
            .unwrap();
        self.state
            .db
            .collection::<crate::models::telegram_bot_request::TelegramBotRequest>(
                crate::models::telegram_bot_request::COLLECTION_NAME,
            )
            .find_one(doc! {"_id": &request.id})
            .await
            .unwrap()
            .unwrap()
    }
}

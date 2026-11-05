//! Metadata-only evidence, captured before returning a successful proxy response.
use super::assistant_acknowledgement_service::ChatAuthority;
use crate::models::{
    assistant_conversation::{ActiveTurn, AssistantConversation},
    assistant_message::WorkEvidence,
    downstream_service::DownstreamService,
    usage_meter::CredentialClass,
};
use mongodb::{Database, bson::doc};

pub struct Signal {
    user: String,
    conversation: String,
    turn: String,
    key: String,
}

pub fn business_service(service: &DownstreamService) -> bool {
    service.service_category == "connection"
        && service.inference.is_none()
        && service.slug != "llm-nyx"
        && !service.slug.starts_with("llm-")
}

impl Signal {
    #[allow(clippy::too_many_arguments)]
    pub fn for_call(
        chat: Option<&ChatAuthority>,
        service: &DownstreamService,
        connection: Option<&str>,
        owner: &str,
        class: CredentialClass,
        auto_connected: bool,
    ) -> Option<Self> {
        let chat = chat?;
        let agent_owner = chat
            .org_agent_access
            .as_ref()
            .map_or(chat.user_id.as_str(), |a| a.owner());
        if !chat.turn_live
            || chat.turn_stopped
            || chat.business_system_success
            || owner != agent_owner
            || (class == CredentialClass::NoAuth && (auto_connected || connection.is_none()))
            || !business_service(service)
            || !matches!(
                class,
                CredentialClass::UserOwned
                    | CredentialClass::NyxidPlatformOauthApp
                    | CredentialClass::AgentOverrideUserOwned
                    | CredentialClass::NodeManaged
                    | CredentialClass::NoAuth
            )
        {
            return None;
        }
        Some(Self {
            user: chat.user_id.clone(),
            conversation: chat.conversation_id.clone(),
            turn: chat.turn_id.clone()?,
            key: chat.api_key_id.clone(),
        })
    }
    pub async fn success(&self, db: &Database) {
        // _id point write; later requests reuse the existing authority snapshot's flag.
        // Never fail a successful business operation because analytics is unavailable.
        if !matches!(tokio::time::timeout(std::time::Duration::from_secs(2), async {
            db.collection::<mongodb::bson::Document>("assistant_conversations").update_one(
            doc!{"_id":&self.conversation,"user_id":&self.user,"credential_api_key_id":&self.key,
                "active_turn.turn_id":&self.turn,"active_turn.business_system_success":{"$ne":true}},
            doc!{"$set":{"active_turn.business_system_success":true}},
        ).await
        }).await, Ok(Ok(_))) {
            tracing::warn!("Assistant business-success signal could not be recorded");
        }
    }
}

pub fn evidence(
    row: &AssistantConversation,
    turn: &ActiveTurn,
    settled_at: Option<chrono::DateTime<chrono::Utc>>,
) -> WorkEvidence {
    WorkEvidence {
        settled_at,
        owner_id: row
            .agent_owner_id
            .clone()
            .unwrap_or_else(|| row.user_id.clone()),
        source: if turn.trigger_run_id.is_some() {
            "scheduled"
        } else if turn.channel_event_id.is_some()
            || turn.platform_origin.is_some()
            || turn.channel_adapter_origin.is_some()
            || turn.origin == crate::models::assistant_conversation::TurnOrigin::Channel
        {
            "channel"
        } else if turn.origin == crate::models::assistant_conversation::TurnOrigin::Event {
            "event"
        } else {
            "chat"
        }
        .into(),
        business_system_success: turn.business_system_success,
        trigger_run_id: turn.trigger_run_id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{services::service_insights_activity::RequestAudit, test_utils};
    use mongodb::bson::Document;

    fn chat() -> ChatAuthority {
        ChatAuthority {
            execution_group_id: None,
            execution_unsupported: false,
            execution_voice_task: false,
            execution_task: None,
            execution_permission: None,
            business_system_success: false,
            collaboration_thread: false,
            channel_adapter_origin: None,
            invocation_controlled: false,
            invocation_id: None,
            platform_thread: false,
            platform_origin: None,
            org_agent_access: None,
            turn_id: Some("turn".into()),
            turn_stopped: false,
            turn_live: true,
            machine_node_ids: vec![],
            saved_login_ids: vec![],
            conversation_id: "thread".into(),
            user_id: uuid::Uuid::new_v4().to_string(),
            api_key_id: "key".into(),
            role: crate::models::assistant_conversation::AgentRole::Orchestrator,
            agent_id: "agent".into(),
            agent_name: "NyxBot".into(),
            guest: false,
            confirmation_policy: None,
            report_to: None,
            with_nyxbot: false,
        }
    }
    fn business() -> DownstreamService {
        let mut service = test_utils::test_auto_connected_catalog_service();
        service.service_category = "connection".into();
        service.slug = "business".into();
        service.inference = None;
        service
    }
    #[test]
    fn real_work_requires_owner_business_authority_and_excludes_platform_and_models() {
        let mut c = chat();
        let mut svc = business();
        for class in [
            CredentialClass::UserOwned,
            CredentialClass::AgentOverrideUserOwned,
            CredentialClass::NodeManaged,
            CredentialClass::NyxidPlatformOauthApp,
            CredentialClass::NoAuth,
        ] {
            assert!(
                Signal::for_call(Some(&c), &svc, Some("connection"), &c.user_id, class, false)
                    .is_some(),
                "{class:?}"
            );
        }
        assert!(
            Signal::for_call(
                Some(&c),
                &svc,
                Some("connection"),
                &c.user_id,
                CredentialClass::NyxidManagedMaster,
                false
            )
            .is_none()
        );
        assert!(
            Signal::for_call(
                Some(&c),
                &svc,
                Some("connection"),
                &c.user_id,
                CredentialClass::NoAuth,
                true
            )
            .is_none()
        );
        assert!(
            Signal::for_call(
                Some(&c),
                &svc,
                Some("connection"),
                "other-owner",
                CredentialClass::UserOwned,
                false
            )
            .is_none()
        );
        for category in ["internal", "provider"] {
            svc.service_category = category.into();
            assert!(!business_service(&svc));
        }
        svc.service_category = "connection".into();
        svc.slug = "llm-nyx".into();
        assert!(!business_service(&svc));
        svc.slug = "business".into();
        svc.inference = Some(crate::models::downstream_service::ServiceInference {
            wire_protocol:
                crate::models::downstream_service::InferenceWireProtocol::OpenaiResponses,
            model_list: false,
            realtime: false,
            voice: None,
        });
        assert!(!business_service(&svc));
        svc.inference = None;
        c.business_system_success = true;
        assert!(
            Signal::for_call(
                Some(&c),
                &svc,
                Some("connection"),
                &c.user_id,
                CredentialClass::UserOwned,
                false
            )
            .is_none()
        );
        c.business_system_success = false;
        c.turn_live = false;
        assert!(
            Signal::for_call(
                Some(&c),
                &svc,
                Some("connection"),
                &c.user_id,
                CredentialClass::UserOwned,
                false
            )
            .is_none()
        );
        c.turn_live = true;
        c.turn_stopped = true;
        assert!(
            Signal::for_call(
                Some(&c),
                &svc,
                Some("connection"),
                &c.user_id,
                CredentialClass::UserOwned,
                false
            )
            .is_none()
        );
    }
    #[tokio::test]
    async fn real_work_response_records_only_2xx_and_fences_replaced_turns() {
        let db = test_utils::connect_transaction_test_database("real_work_signal").await;
        let c = chat();
        let svc = business();
        let rows = db.collection::<Document>("assistant_conversations");
        rows.insert_one(doc!{"_id":&c.conversation_id,"user_id":&c.user_id,"credential_api_key_id":&c.api_key_id,"active_turn":{"turn_id":"turn"}}).await.unwrap();
        let mut auth = test_utils::test_auth_user(&c.user_id);
        auth.assistant_chat = Some(std::sync::Arc::new(c.clone()));
        for status in [101, 302, 400, 503] {
            let mut audit = RequestAudit::new(
                &db,
                &auth,
                Some("connection"),
                &svc.id,
                &c.user_id,
                "request",
                CredentialClass::UserOwned,
            );
            audit.business_service(Some(&c), &svc, false);
            audit.response(status).await;
            let saved = rows.find_one(doc! {}).await.unwrap().unwrap();
            assert!(
                !saved
                    .get_document("active_turn")
                    .unwrap()
                    .get_bool("business_system_success")
                    .unwrap_or(false)
            );
        }
        let mut audit = RequestAudit::new(
            &db,
            &auth,
            Some("connection"),
            &svc.id,
            &c.user_id,
            "request",
            CredentialClass::UserOwned,
        );
        audit.business_service(Some(&c), &svc, false);
        audit.response(201).await;
        assert!(
            rows.find_one(doc! {})
                .await
                .unwrap()
                .unwrap()
                .get_document("active_turn")
                .unwrap()
                .get_bool("business_system_success")
                .unwrap()
        );
        rows.update_one(
            doc! {},
            doc! {"$set":{"active_turn":{"turn_id":"replacement"}}},
        )
        .await
        .unwrap();
        Signal::for_call(
            Some(&c),
            &svc,
            Some("connection"),
            &c.user_id,
            CredentialClass::UserOwned,
            false,
        )
        .unwrap()
        .success(&db)
        .await;
        assert!(
            rows.find_one(doc! {})
                .await
                .unwrap()
                .unwrap()
                .get_document("active_turn")
                .unwrap()
                .get("business_system_success")
                .is_none()
        );
        db.drop().await.unwrap();
    }
}

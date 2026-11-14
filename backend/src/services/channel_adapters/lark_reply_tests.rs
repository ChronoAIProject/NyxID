use super::*;
use crate::services::channel_platform::BotCredentials;
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, method, path, query_param},
};

async fn adapter(platform: &str) -> (MockServer, LarkFamilyAdapter) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/open-apis/auth/v3/tenant_access_token/internal"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"code":0,"tenant_access_token":"tenant-token","expire":7200}),
            ),
        )
        .mount(&server)
        .await;
    let adapter = LarkFamilyAdapter::media_test_adapter(&server.uri(), platform);
    (server, adapter)
}
fn credentials() -> BotCredentials<'static> {
    BotCredentials {
        billing: None,
        token: "app:secret",
        platform_bot_id: None,
        platform_secrets: None,
    }
}
fn reply(source: Option<&str>) -> OutboundReply {
    OutboundReply {
        attachments: vec![],
        text: Some("The conference starts at 15:00 SGT.".into()),
        reply_to_platform_message_id: source.map(str::to_owned),
        metadata: None,
    }
}

#[tokio::test]
async fn plain_replies_quote_the_exact_message_and_notices_are_independent() {
    for platform in ["lark", "feishu"] {
        let (server, adapter) = adapter(platform).await;
        Mock::given(method("GET")).and(path("/open-apis/im/v1/messages/om_question"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"code":0,"data":{"items":[{"message_id":"om_question","chat_id":"oc_guest"}]}})))
            .expect(1).mount(&server).await;
        Mock::given(method("POST")).and(path("/open-apis/im/v1/messages/om_question/reply"))
            .and(body_json(json!({"msg_type":"text","content":"{\"text\":\"The conference starts at 15:00 SGT.\"}","reply_in_thread":false})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"code":0,"data":{"message_id":"om_answer"}})))
            .expect(1).mount(&server).await;
        Mock::given(method("POST")).and(path("/open-apis/im/v1/messages"))
            .and(query_param("receive_id_type","chat_id"))
            .and(body_json(json!({"receive_id":"oc_guest","msg_type":"text","content":"{\"text\":\"The conference starts at 15:00 SGT.\"}"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"code":0,"data":{"message_id":"om_notice"}})))
            .expect(1).mount(&server).await;
        let http = reqwest::Client::new();
        assert_eq!(
            adapter
                .send_reply(
                    &http,
                    &credentials(),
                    "oc_guest",
                    &reply(Some("om_question"))
                )
                .await
                .unwrap()
                .as_deref(),
            Some("om_answer")
        );
        assert_eq!(
            adapter
                .send_reply(&http, &credentials(), "oc_guest", &reply(None))
                .await
                .unwrap()
                .as_deref(),
            Some("om_notice")
        );
    }
}

#[tokio::test]
async fn wrong_chat_deleted_and_invalid_reply_sources_never_send() {
    for platform in ["lark", "feishu"] {
        let (server, adapter) = adapter(platform).await;
        for (id, chat, deleted) in [
            ("om_foreign", "oc_other", false),
            ("om_deleted", "oc_guest", true),
        ] {
            Mock::given(method("GET")).and(path(format!("/open-apis/im/v1/messages/{id}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"code":0,"data":{"items":[{"message_id":id,"chat_id":chat,"deleted":deleted}]}})))
                .expect(1).mount(&server).await;
            assert!(
                adapter
                    .send_reply(
                        &reqwest::Client::new(),
                        &credentials(),
                        "oc_guest",
                        &reply(Some(id))
                    )
                    .await
                    .is_err()
            );
        }
        assert!(
            adapter
                .send_reply(
                    &reqwest::Client::new(),
                    &credentials(),
                    "oc_guest",
                    &reply(Some("../foreign"))
                )
                .await
                .is_err()
        );
        let requests = server.received_requests().await.unwrap();
        assert!(
            !requests
                .iter()
                .any(|r| r.method == "POST" && r.url.path().contains("/im/"))
        );
    }
}

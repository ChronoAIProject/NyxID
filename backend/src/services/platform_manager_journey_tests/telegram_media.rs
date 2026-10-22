use super::*;
use crate::services::feature_flag_service as flags;
use wiremock::matchers::query_param;

#[tokio::test]
async fn manager_file_failures_and_disabled_media_preserve_the_owner_text_turn() {
    fn journey() -> futures::future::BoxFuture<'static, Journey> {
        Box::pin(Journey::new())
    }
    for case in ["disabled", "oversize", "partial"] {
        let j = journey().await;
        j.link().await;
        if case != "disabled" {
            flags::set_platform_override(
                &j.state.db,
                flags::ASSISTANT_CHANNEL_MEDIA_FLAG_KEY,
                &flags::FlagTarget::User(j.owner.clone()),
                true,
                &j.owner,
            )
            .await
            .unwrap();
        }
        let mut update = j.update(
            9200,
            "Keep this forwarded question even if its file fails",
            true,
        );
        update["message"]["document"] = json!({"file_id":"failed", "file_name":"failed.txt", "file_size": if case == "oversize" { 64 * 1024 * 1024 } else { 12 }});
        if case == "partial" {
            update["message"]["external_reply"] =
                json!({"document":{"file_id":"success","file_name":"success.txt"}});
            Mock::given(method("GET"))
                .and(path("/bot100:manager-test-secret/getFile"))
                .and(query_param("file_id", "success"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{"file_path":"documents/success.txt","file_size":12}})))
                .expect(1).mount(&j.server).await;
            Mock::given(method("GET"))
                .and(path(
                    "/file/bot100:manager-test-secret/documents/success.txt",
                ))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(b"Full content".to_vec()))
                .expect(1)
                .mount(&j.server)
                .await;
        }
        j.webhook(&update).await;
        let (_, reply, _) = j.saved().await;
        let messages =
            engine::messages(&j.state.db, &j.owner, &reply.turn.conversation_id, 50, None)
                .await
                .unwrap();
        let message = messages.iter().find(|m| m.role == "user").unwrap();
        assert!(
            message
                .text
                .contains("Keep this forwarded question even if its file fails"),
            "{case}"
        );
        let expected_note = match case {
            "disabled" => "attachments are not shared",
            "oversize" => "too large: 1",
            _ => "unavailable: 1",
        };
        assert!(
            message.text.contains(expected_note),
            "{case}: {}",
            message.text
        );
        assert_eq!(message.attachments.len(), usize::from(case == "partial"));
        assert_eq!(j.egress.0.load(Ordering::SeqCst), 1);
        let input = j.egress.1.lock().await;
        assert!(input[0].to_string().contains(expected_note));
        drop(input);
        if case != "partial" {
            assert!(
                !j.server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .any(|r| r.url.path().ends_with("/getFile"))
            );
        }
        j.state.db.drop().await.unwrap();
    }
}

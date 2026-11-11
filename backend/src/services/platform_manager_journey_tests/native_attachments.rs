use super::*;

struct AttachmentReply {
    state: AppState,
    inner: Arc<dyn ManagerTransportProducer>,
    text: &'static str,
}

#[async_trait]
impl ManagerTransportProducer for AttachmentReply {
    async fn admit_manager_update(
        &self,
        db: &Database,
        update: &VerifiedManagerUpdate,
    ) -> AppResult<IngressAdmission> {
        self.inner.admit_manager_update(db, update).await
    }

    async fn deliver_platform_reply(
        &self,
        db: &Database,
        destination: &ReplyDestination,
        reference: &DeliveryRef,
        content: ReplyContent,
    ) -> AppResult<DeliveryOutcome> {
        let content = if let DeliveryRef::Reply(reply) = reference {
            // Seed a saved tool result before production rendering and delivery.
            // Native transport has no attachment-upload contract for this file.
            db.collection::<Document>(MESSAGES)
                .update_one(
                    doc! {"_id":&reply.reply_id},
                    doc! {"$set":{"text":self.text,"attachments":[{
                        "id":"report", "origin":"tool", "content_type":"application/pdf",
                        "size":100_i64,"label":"Report","chat_delivery":true,
                    }]}},
                )
                .await?;
            let saved = db
                .collection::<AssistantMessage>(MESSAGES)
                .find_one(doc! {"_id":&reply.reply_id})
                .await?
                .unwrap();
            ReplyContent::Text(delivery::render_personal(&self.state, &saved, None).await?)
        } else {
            content
        };
        self.inner
            .deliver_platform_reply(db, destination, reference, content)
            .await
    }
}

#[test]
fn native_attachment_only_and_mixed_replies_deliver_the_exact_saved_thread_fallback() {
    crate::test_utils::run_native_journey_test("native_attachment_fallback", || async {
        for text in ["", "Here is the complete report."] {
            let mut j = NativeJourney::new().await;
            let inner = j.producers.transport.clone();
            Arc::get_mut(&mut j.producers).unwrap().transport = Arc::new(AttachmentReply {
                state: j.state.clone(),
                inner,
                text,
            });
            j.dispatch(j.admission(1, "login", None).await).await;
            let requests = j.server.received_requests().await.unwrap();
            let login: Value = serde_json::from_slice(&requests[0].body).unwrap();
            let url = reqwest::Url::parse(
                login["content"]
                    .as_str()
                    .unwrap()
                    .split_whitespace()
                    .find(|word| word.starts_with("http"))
                    .unwrap(),
            )
            .unwrap();
            let token = url
                .fragment()
                .unwrap()
                .strip_prefix("intent_token=")
                .unwrap();
            let review = j
                .browser(
                    "POST",
                    "/api/v1/platform-personal/review",
                    Some(json!({"intent_token":token})),
                )
                .await;
            tokio::time::sleep(std::time::Duration::from_millis(800)).await;
            let approved = j
                .browser(
                    "POST",
                    "/api/v1/platform-personal/approve",
                    Some(json!({"challenge_id":review["challenge_id"],
                    "review_digest":review["review_digest"],"replace_existing":false})),
                )
                .await;
            let code = approved["confirmation_command"]
                .as_str()
                .unwrap()
                .strip_prefix("/nyx confirm code:")
                .unwrap();
            j.dispatch(j.admission(2, "confirm", Some(("code", code))).await)
                .await;
            j.dispatch(
                j.admission(3, "ask", Some(("question", "Prepare a report")))
                    .await,
            )
            .await;
            let (reply, outcome) =
                tokio::time::timeout(std::time::Duration::from_secs(30), j.finished.recv())
                    .await
                    .unwrap()
                    .unwrap();
            assert!(matches!(outcome, DeliveryOutcome::NativeSent { .. }));
            let requests = j.server.received_requests().await.unwrap();
            let sent: Value = serde_json::from_slice(&requests.last().unwrap().body).unwrap();
            let body = sent["content"].as_str().unwrap();
            let displayed: String = pulldown_cmark::Parser::new(body)
                .filter_map(|event| match event {
                    pulldown_cmark::Event::Text(text) | pulldown_cmark::Event::Code(text) => {
                        Some(text.to_string())
                    }
                    _ => None,
                })
                .collect();
            assert!(displayed.starts_with(text), "{body}");
            assert!(
                displayed.contains("This chat cannot receive these attachments."),
                "{body}"
            );
            let expected_url = format!(
                "{}/assistant?c={}",
                j.state.config.frontend_url.trim_end_matches('/'),
                reply.turn.conversation_id
            );
            assert!(pulldown_cmark::Parser::new(body).any(|event| matches!(event,
                pulldown_cmark::Event::Start(pulldown_cmark::Tag::Link { dest_url, .. }) if dest_url.as_ref() == expected_url.as_str()
            )), "Fallback must be a usable exact-thread link: {body}");
            assert!(
                requests
                    .last()
                    .unwrap()
                    .url
                    .path()
                    .starts_with("/webhooks/100/native-journey-3")
            );
            assert_eq!(
                requests.len(),
                3,
                "login, confirmation and one complete result"
            );
            j.state.db.drop().await.unwrap();
        }
    });
}

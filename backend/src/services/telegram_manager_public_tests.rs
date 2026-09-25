//! Manager-as-public-channel behavior: welcome, addressed commands, claim privacy.
//! Nested under `telegram_new_tests` so it shares that module's fixtures.

use super::*;
use crate::services::telegram_new_claims::{CLAIM_CODE_REDACTED, scrub_claim_codes};

async fn sent_messages(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.url.path().ends_with("/sendMessage"))
        .filter_map(|request| request.body_json::<Value>().ok())
        .collect()
}

async fn challenge_for(service: &TelegramNewService<'_>, actor: &str) -> String {
    let (_, link) = service.begin(actor, actor, "Support", true).await.unwrap();
    reqwest::Url::parse(&link)
        .unwrap()
        .query_pairs()
        .find(|(key, _)| key == "start")
        .unwrap()
        .1
        .to_string()
}

async fn relayed(
    service: &TelegramNewService<'_>,
    headers: &HeaderMap,
    update: Value,
) -> Option<Value> {
    service
        .webhook(headers, &serde_json::to_vec(&update).unwrap())
        .await
        .unwrap()
        .map(|(_, update)| update)
}

#[tokio::test]
async fn telegram_manager_public_start_welcomes_chat_without_hiding_creation() {
    let (state, actor, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;

    // Without a channel connection the manager only offers creation.
    assert!(
        relayed(&service, &headers, message(json!({"text": "/start"})))
            .await
            .is_none()
    );
    let messages = sent_messages(&server).await;
    assert_eq!(messages.len(), 2);
    let creation_only = &messages[0];
    assert!(creation_only.get("reply_markup").is_none());
    assert!(
        creation_only["text"]
            .as_str()
            .unwrap()
            .starts_with("<b>Create your Telegram bot</b>")
    );

    register_manager_channel(&state, &actor, &server).await;
    for start in ["/start", "/start@NyxSetupBot", " /start@nyxsetupbot "] {
        let before = sent_messages(&server).await.len();
        assert!(
            relayed(&service, &headers, message(json!({"text": start})))
                .await
                .is_none()
        );
        let messages = sent_messages(&server).await;
        assert_eq!(messages.len(), before + 2);
        let welcome = &messages[before];
        assert_eq!(welcome["chat_id"], 700);
        assert!(welcome.get("reply_markup").is_none());
        let text = welcome["text"].as_str().unwrap();
        assert!(text.starts_with("<b>Welcome</b>"), "{text}");
        assert!(text.contains("Send a message here to chat"));
        assert!(text.contains("/recover @YourBotUsername"));
        assert!(text.contains("latest Telegram app"));
        let keyboard = &messages[before + 1];
        assert_eq!(keyboard["chat_id"], 700);
        assert!(keyboard["reply_markup"]["keyboard"][0][0]["request_managed_bot"].is_object());
    }

    // A private website handoff still takes precedence over the public welcome.
    let challenge = challenge_for(&service, &actor).await;
    assert!(
        relayed(
            &service,
            &headers,
            message(json!({"text": format!("/start {challenge}")}))
        )
        .await
        .is_none()
    );
    assert!(
        relayed(&service, &headers, message(json!({"text": "/start"})))
            .await
            .is_none()
    );
    let messages = sent_messages(&server).await;
    let personal = &messages[messages.len() - 2];
    assert!(
        personal["text"]
            .as_str()
            .unwrap()
            .contains("Only create a bot here if you started this setup")
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_manager_addressed_commands_follow_the_configured_username() {
    let (state, actor, server) = fixture().await;
    let channel = register_manager_channel(&state, &actor, &server).await;
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;

    // Addressed to the manager: the deep link binds the sender exactly as the bare form does.
    let challenge = challenge_for(&service, &actor).await;
    assert!(
        relayed(
            &service,
            &headers,
            message(json!({"text": format!("/start@NyxSetupBot {challenge}")})),
        )
        .await
        .is_none()
    );
    let request = state
        .db
        .collection::<TelegramBotRequest>(REQUESTS)
        .find_one(doc! {"actor_user_id": &actor, "active": true})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.status, Status::WaitingBot);
    assert_eq!(request.telegram_user_id, Some(700));
    assert_eq!(request.start_update_id, Some(1));

    // Addressed recovery is handled by the setup flow, never relayed.
    let before = sent_messages(&server).await.len();
    assert!(
        relayed(
            &service,
            &headers,
            message(json!({"text": "/recover@NYXSETUPBOT @CustomerBot"}))
        )
        .await
        .is_none()
    );
    let reply = sent_messages(&server).await;
    assert_eq!(reply.len(), before + 1);
    assert!(
        reply.last().unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("No recent, unconnected bot creation")
    );

    // Misaddressed setup commands may still contain a private challenge.
    for text in [
        "/start@OtherBot private-challenge",
        "/recover@OtherBot @CustomerBot",
    ] {
        assert!(
            relayed(&service, &headers, message(json!({"text": text})))
                .await
                .is_none()
        );
    }
    for text in ["/started", "please /start over"] {
        let update = relayed(&service, &headers, message(json!({"text": text})))
            .await
            .unwrap_or_else(|| panic!("{text} must be relayed as ordinary chat"));
        assert_eq!(update["message"]["text"], text);
    }
    assert_eq!(sent_messages(&server).await.len(), before + 1);
    assert!(
        service
            .manager_channel(100)
            .await
            .unwrap()
            .is_some_and(|bot| bot.id == channel.id)
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_manager_claim_shaped_text_is_scrubbed_before_routing() {
    let (state, actor, server) = fixture().await;
    register_manager_channel(&state, &actor, &server).await;
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;

    let hyphenated = relayed(
        &service,
        &headers,
        message(json!({
            "text": "my code is ABCDE-FGHJK-LMNPQ-RSTUV, thanks",
            "entities": [{"type": "code", "offset": 11, "length": 23}],
        })),
    )
    .await
    .unwrap();
    assert_eq!(
        hyphenated["message"]["text"],
        format!("my code is {CLAIM_CODE_REDACTED}, thanks")
    );
    assert!(hyphenated["message"].get("entities").is_none());

    let caption = relayed(
        &service,
        &headers,
        message(json!({
            "photo": [{"file_id": "photo-1", "file_unique_id": "u1", "width": 1, "height": 1}],
            "caption": "ABCDE FGHJK LMNPQ RSTUV",
            "caption_entities": [{"type": "bold", "offset": 0, "length": 5}],
        })),
    )
    .await
    .unwrap();
    assert_eq!(caption["message"]["caption"], CLAIM_CODE_REDACTED);
    assert!(caption["message"].get("caption_entities").is_none());

    let plain = relayed(
        &service,
        &headers,
        message(json!({
            "text": "hello world, these are plenty normal words ABCDE FGHJK",
            "entities": [{"type": "bold", "offset": 0, "length": 5}],
        })),
    )
    .await
    .unwrap();
    assert_eq!(
        plain["message"]["text"],
        "hello world, these are plenty normal words ABCDE FGHJK"
    );
    assert!(plain["message"]["entities"].is_array());

    let nested = relayed(&service, &headers, message(json!({
        "pinned_message": {"message_id": 20, "text": "/start private-start-challenge", "reply_markup": {"inline_keyboard": [[{"url": "https://app.nyxid.test/?claim=secret"}]]}},
        "giveaway_completed": {"giveaway_message": {"message_id": 21, "text": "private-claim", "quote": {"text": "private-quote"}}},
    }))).await.unwrap();
    assert_eq!(
        nested["message"]["pinned_message"],
        json!({"message_id": 20})
    );
    assert_eq!(
        nested["message"]["giveaway_completed"]["giveaway_message"],
        json!({"message_id": 21})
    );
    assert!(!nested.to_string().contains("private-"));

    for body in [
        json!({"text": "https://app.nyxid.test/channel-bots?claim=ABCDE-FGHJK-LMNPQ-RSTUV&claim_entry=true"}),
        json!({"text": "Open my claim", "entities": [{"type": "text_link", "offset": 0, "length": 13, "url": "https://app.nyxid.test/?claim=ABCDEFGHJKLMNPQRSTUV"}]}),
        json!({"text": "Open my claim", "entities": [{"type": "text_link", "offset": 0, "length": 13, "url": "https://app.nyxid.test/?%63laim=ABCDE%2DFGHJK%2DLMNPQ%2DRSTUV"}]}),
        json!({"caption": "setup", "caption_entities": [{"type": "text_link", "offset": 0, "length": 5, "url": "https://app.nyxid.test/?claim=ABCDE-FGHJK-LMNPQ-RSTUV"}]}),
        json!({"text": "setup", "reply_markup": {"inline_keyboard": [[{"text": "Connect", "url": "https://app.nyxid.test/?claim=ABCDE-FGHJK-LMNPQ-RSTUV"}]]}}),
    ] {
        let update = relayed(&service, &headers, message(body)).await.unwrap();
        assert!(!update.to_string().contains("ABCDE-FGHJK-LMNPQ-RSTUV"));
        assert!(!update.to_string().contains("ABCDEFGHJKLMNPQRSTUV"));
        assert!(!update.to_string().contains("ABCDE%2D"));
    }
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_manager_claim_message_is_protected_and_matches_the_scrubber() {
    let (state, _actor, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;
    webhook(
        &service,
        &headers,
        message(json!({"managed_bot_created": {"bot": bot()}})),
    )
    .await;
    let claim = sent_messages(&server)
        .await
        .into_iter()
        .find(|body| {
            body["text"]
                .as_str()
                .is_some_and(|text| text.contains("Claim code:"))
        })
        .expect("creation without a website request mints a claim");
    assert_eq!(claim["protect_content"], true);
    let text = claim["text"].as_str().unwrap();
    let scrubbed = scrub_claim_codes(text).expect("the delivered code shape must be scrubbable");
    assert!(scrubbed.contains(&format!("Claim code: {CLAIM_CODE_REDACTED}")));
    let code = text
        .lines()
        .find_map(|line| line.strip_prefix("Claim code: "))
        .unwrap();
    assert!(!scrubbed.contains(code));
    state.db.drop().await.unwrap();
}

#[test]
fn scrub_claim_codes_matches_only_exact_code_shapes() {
    let redacted = CLAIM_CODE_REDACTED;
    assert_eq!(
        scrub_claim_codes("code: ABCDE-FGHJK-LMNPQ-RSTUV.").as_deref(),
        Some(format!("code: {redacted}.").as_str())
    );
    assert_eq!(
        scrub_claim_codes("(abcdefghjklmnpqrstuv)").as_deref(),
        Some(format!("({redacted})").as_str())
    );
    assert_eq!(
        scrub_claim_codes("ABCDE FGHJK LMNPQ RSTUV and again ABCDE-FGHJK-LMNPQ-RSTUV").as_deref(),
        Some(format!("{redacted} and again {redacted}").as_str())
    );
    assert_eq!(
        scrub_claim_codes("https://app.nyxid.test/?claim=ABCDE%2DFGHJK%2DLMNPQ%2DRSTUV&next=1")
            .as_deref(),
        Some(format!("https://app.nyxid.test/?claim={redacted}&next=1").as_str())
    );
    // Lowercase space groups, wrong lengths, and excluded characters stay untouched.
    for text in [
        "abcde fghjk lmnpq rstuv",
        "ABCDE FGHJK LMNPQ",
        "ABCDE-FGHJK-LMNPQ-RSTU",
        "ABCDE-FGHJK-LMNPQ-RSTUVW",
        "ABCDE-FGHIK-LMNPQ-RSTUV",
        "ABCD0-FGHJK-LMNPQ-RSTUV",
        "these three great cards arrive",
        "",
    ] {
        assert_eq!(scrub_claim_codes(text), None, "{text:?}");
    }
}

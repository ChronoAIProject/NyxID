use super::*;
use crate::services::{
    assistant_upload_service::byte_budget as budget, feature_flag_service as flags,
};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::*};

async fn flag(state: &crate::AppState, user: &str, enabled: bool) {
    flags::set_platform_override(
        &state.db,
        flags::ASSISTANT_CHANNEL_MEDIA_FLAG_KEY,
        &flags::FlagTarget::User(user.into()),
        enabled,
        user,
    )
    .await
    .unwrap();
}

async fn fixture() -> (
    crate::AppState,
    String,
    MockServer,
    crate::models::channel_bot::ChannelBot,
) {
    let (state, owner, server) = crate::services::telegram_new_tests::fixture().await;
    let bot =
        crate::services::telegram_new_tests::register_manager_channel(&state, &owner, &server)
            .await;
    (state, owner, server, bot)
}

fn file(id: &str, size: Option<u64>) -> crate::services::channel_platform::InboundAttachment {
    let mut files = attachments(
        &json!({"document":{"file_id":id,"file_name":format!("{id}.txt"),"mime_type":"text/plain","file_size":size}}),
    );
    files.remove(0)
}

async fn serve(server: &MockServer, id: &str, bytes: &[u8], gets: u64) {
    Mock::given(method("GET"))
        .and(path("/bot100:manager-test-secret/getFile"))
        .and(query_param("file_id", id))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{"file_path":format!("documents/{id}.txt"),"file_size":bytes.len()}})))
        .expect(gets)
        .mount(server).await;
    Mock::given(method("GET"))
        .and(path(format!(
            "/file/bot100:manager-test-secret/documents/{id}.txt"
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.to_vec()))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn owner_flag_blocks_all_downloads_until_that_person_is_enabled() {
    let (state, owner, server, bot) = fixture().await;
    let files = [file("private", None)];
    let other = uuid::Uuid::new_v4().to_string();
    state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &other,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    flag(&state, &other, true).await;
    for enabled in [None, Some(false), Some(true)] {
        if let Some(enabled) = enabled {
            flag(&state, &owner, enabled).await;
        }
        let before = server.received_requests().await.unwrap().len();
        if enabled == Some(true) {
            serve(&server, "private", b"Complete file", 1).await;
        }
        let input = prepare_uploads_from(&state, &bot, &owner, &files, &server.uri()).await;
        if enabled == Some(true) {
            assert_eq!(input.uploads.len(), 1);
            assert!(input.note.is_empty());
        } else {
            assert!(input.uploads.is_empty());
            assert!(input.note.contains("1 attachments are not shared"));
            assert_eq!(server.received_requests().await.unwrap().len(), before);
        }
    }
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn each_failed_file_degrades_without_discarding_successful_siblings_or_retry() {
    let (state, owner, server, bot) = fixture().await;
    flag(&state, &owner, true).await;
    serve(&server, "first", b"First file", 1).await;
    // A falsely small declaration must release the initial reservation and
    // retry under a new full-size reservation, including a new admission.
    serve(&server, "last", b"Complete last file", 2).await;
    let files = [
        file("first", None),
        file("missing", None),
        file("last", Some(1)),
    ];
    let input = prepare_uploads_from(&state, &bot, &owner, &files, &server.uri()).await;
    assert_eq!(input.uploads.len(), 2);
    assert!(input.note.contains("1 attachments could not be read"));
    assert!(input.note.contains("unavailable: 1"));
    assert!(
        !input
            .note
            .contains(crate::services::assistant_channel_media::FAILED_NOTE)
    );
    // No reservation leaks after success, failure, or initial-limit retry.
    let all = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        budget::reserve(budget::CAPACITY / 2),
    )
    .await
    .unwrap()
    .unwrap();
    drop(all);
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn oversize_rate_limit_and_attachment_cap_produce_bounded_failure_notes() {
    use mongodb::bson::doc;
    let (state, owner, server, bot) = fixture().await;
    flag(&state, &owner, true).await;
    let before = server.received_requests().await.unwrap().len();
    let input = prepare_uploads_from(
        &state,
        &bot,
        &owner,
        &[file("large", Some(u64::MAX))],
        &server.uri(),
    )
    .await;
    assert!(input.uploads.is_empty());
    assert!(input.note.contains("too large: 1"));
    assert!(
        input
            .note
            .contains(crate::services::assistant_channel_media::FAILED_NOTE)
    );
    assert_eq!(server.received_requests().await.unwrap().len(), before);
    let count = crate::services::assistant_nyxagent::MAX_TURN_ATTACHMENTS;
    let mut files = Vec::new();
    for index in 0..count + 2 {
        let id = format!("file{index}");
        files.push(file(&id, None));
        if index < count {
            serve(&server, &id, b"A file", 1).await;
        }
    }
    let input = prepare_uploads_from(&state, &bot, &owner, &files, &server.uri()).await;
    assert_eq!(input.uploads.len(), count);
    assert!(input.note.contains("2 attachments could not be read"));
    assert!(input.note.contains("unsupported or attachment limit: 2"));
    let minute = chrono::Utc::now().timestamp() / 60;
    state
        .db
        .collection::<mongodb::bson::Document>("assistant_upload_limits")
        .update_one(
            doc! {"_id":format!("{owner}:{minute}")},
            doc! {"$set":{"attempts":30}},
        )
        .upsert(true)
        .await
        .unwrap();
    let before = server.received_requests().await.unwrap().len();
    let input = prepare_uploads_from(
        &state,
        &bot,
        &owner,
        &[file("limited", None)],
        &server.uri(),
    )
    .await;
    assert!(input.uploads.is_empty());
    assert!(input.note.contains("unavailable: 1"));
    assert_eq!(server.received_requests().await.unwrap().len(), before);
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_shares_browser_byte_budget_and_releases_on_cancellation() {
    let (state, owner, server, bot) = fixture().await;
    flag(&state, &owner, true).await;
    let all = budget::reserve(budget::CAPACITY / 2).await.unwrap();
    let before = server.received_requests().await.unwrap().len();
    let input =
        prepare_uploads_from(&state, &bot, &owner, &[file("busy", None)], &server.uri()).await;
    assert!(input.uploads.is_empty());
    assert!(input.note.contains("unavailable: 1"));
    assert_eq!(server.received_requests().await.unwrap().len(), before);
    drop(all);
    Mock::given(method("GET"))
        .and(path("/bot100:manager-test-secret/getFile"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(5)))
        .mount(&server)
        .await;
    let task = tokio::spawn({
        let state = state.clone();
        let bot = bot.clone();
        let owner = owner.clone();
        let base = server.uri();
        async move {
            prepare_uploads_from(&state, &bot, &owner, &[file("cancelled", None)], &base).await
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path().ends_with("/getFile"))
        {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(20),
            budget::reserve(budget::CAPACITY / 2)
        )
        .await
        .is_err()
    );
    task.abort();
    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
    let all = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        budget::reserve(budget::CAPACITY / 2),
    )
    .await
    .unwrap()
    .unwrap();
    drop(all);
    state.db.drop().await.unwrap();
}

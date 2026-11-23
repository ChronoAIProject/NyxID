//! Real upload ingress -> extraction -> turn input -> chat-key MCP reads.
use super::*;
use std::io::{Cursor, Write};

struct FileFixture {
    name: &'static str,
    mime: &'static str,
    bytes: Vec<u8>,
    text: Option<String>,
}

fn pdf() -> Vec<u8> {
    use lopdf::{
        Document, Object, Stream,
        content::{Content, Operation},
        dictionary,
    };
    let mut document = Document::with_version("1.5");
    let pages = document.new_object_id();
    let font = document
        .add_object(dictionary! {"Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica"});
    let resources = document.add_object(dictionary! {"Font"=>dictionary! {"F1"=>font}});
    let mut ids = Vec::new();
    for text in ["First PDF page", "Second PDF page"] {
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![20.into(), 100.into()]),
                Operation::new("Tj", vec![Object::string_literal(text)]),
                Operation::new("ET", vec![]),
            ],
        }
        .encode()
        .unwrap();
        let stream = document.add_object(Stream::new(dictionary! {}, content));
        let page = document.add_object(dictionary! {"Type"=>"Page", "Parent"=>pages,
        "MediaBox"=>vec![0.into(),0.into(),200.into(),200.into()],
        "Resources"=>resources, "Contents"=>stream});
        ids.push(Object::Reference(page));
    }
    document.objects.insert(
        pages,
        Object::Dictionary(dictionary! {"Type"=>"Pages", "Kids"=>ids, "Count"=>2}),
    );
    let root = document.add_object(dictionary! {"Type"=>"Catalog", "Pages"=>pages});
    document.trailer.set("Root", root);
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

fn docx() -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, text) in [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
        ),
        (
            "word/document.xml",
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Document text &amp; details.</w:t></w:r></w:p></w:body></w:document>"#,
        ),
    ] {
        writer.start_file(name, options).unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn fixtures() -> Vec<FileFixture> {
    let mut files = Vec::new();
    for (name, mime, format) in [
        ("photo.png", "image/png", image::ImageFormat::Png),
        ("photo.jpg", "image/jpeg", image::ImageFormat::Jpeg),
        ("photo.gif", "image/gif", image::ImageFormat::Gif),
        ("photo.webp", "image/webp", image::ImageFormat::WebP),
    ] {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            2,
            2,
            image::Rgb([40, 90, 160]),
        ))
        .write_to(&mut bytes, format)
        .unwrap();
        files.push(FileFixture {
            name,
            mime,
            bytes: bytes.into_inner(),
            text: None,
        });
    }
    files.push(FileFixture {
        name: "report.pdf",
        mime: "application/pdf",
        bytes: pdf(),
        text: Some("First PDF page".into()),
    });
    files.push(FileFixture {
        name: "report.docx",
        mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        bytes: docx(),
        text: Some("Document text & details.\n".into()),
    });
    for (name, mime, text) in [
        (
            "data.json",
            "application/json",
            "{\n  \"topic\": \"JSON document\"\n}\n",
        ),
        ("table.csv", "text/csv", "name,value\r\nalpha,123\r\n"),
        (
            "notes.md",
            "text/markdown",
            "# Markdown\n\n**Original** text.\n",
        ),
        ("notes.txt", "text/plain", "Plain text, unchanged.\n"),
    ] {
        files.push(FileFixture {
            name,
            mime,
            bytes: text.as_bytes().to_vec(),
            text: Some(text.into()),
        });
    }
    files
}

async fn draft_id(state: &AppState) -> String {
    super::super::super::assistant_uploads::draft(
        State(state.clone()),
        test_auth_user(OWNER),
        Json(super::super::super::assistant_uploads::Draft { agent_id: None }),
    )
    .await
    .unwrap()
    .0["id"]
        .as_str()
        .unwrap()
        .into()
}

async fn upload_file(state: &AppState, id: &str, file: &FileFixture, browser_mime: &str) -> String {
    let mut request = Request::builder().method("POST").header(
        "x-attachment-name",
        urlencoding::encode(file.name).into_owned(),
    );
    if !browser_mime.is_empty() {
        request = request.header("content-type", browser_mime);
    }
    let (status, Json(item)) = super::super::super::assistant_uploads::upload(
        State(state.clone()),
        test_auth_user(OWNER),
        axum::extract::Path(id.into()),
        request.body(Body::from(file.bytes.clone())).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(status, StatusCode::CREATED, "{}", file.name);
    assert_eq!(item.content_type, file.mime, "{}", file.name);
    assert_eq!(item.size, file.bytes.len() as i64);
    item.id
}

async fn read_document(
    state: &AppState,
    credential: &AssistantCredential,
    id: &str,
    offset: usize,
) -> Value {
    let request = Request::builder().method("POST").uri("/mcp")
        .header("x-api-key", credential.raw_key.as_str())
        .header("content-type", "application/json")
        .body(Body::from(json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{
            "name":"nyx__attachment_read", "arguments":{"attachment_id":id,"offset":offset,"limit":6000}
        }}).to_string())).unwrap();
    let response = Box::pin(super::super::super::mcp_transport::mcp_post(
        State(state.clone()),
        axum::Extension(BillingRoutePolicy::Metered(
            crate::services::billing::BillingIngress::Mcp,
        )),
        request,
    ))
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 100_000)
        .await
        .unwrap();
    let rpc: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(rpc.get("error").is_none());
    let text = rpc["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.chars().count() < 10_000);
    let value: Value = serde_json::from_str(text).unwrap();
    assert_eq!(
        rpc["result"]["isError"],
        value["error"] == "attachment_has_no_extracted_text"
    );
    value
}

async fn read_image(
    state: &AppState,
    credential: &AssistantCredential,
    id: &str,
    file: &FileFixture,
) {
    use tower::ServiceExt;
    let router = Router::new()
        .route(
            "/api/v1/assistant-attachments/{id}/content",
            axum::routing::get(super::super::super::assistant_uploads::thread_image),
        )
        .with_state(state.clone());
    let response = router
        .oneshot(
            Request::builder()
                .uri(format!("/api/v1/assistant-attachments/{id}/content"))
                .header("x-api-key", credential.raw_key.as_str())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], file.mime);
    let bytes = axum::body::to_bytes(response.into_body(), 100_000)
        .await
        .unwrap();
    assert!(bytes.as_ref() == file.bytes.as_slice());
}

#[allow(clippy::too_many_arguments)]
async fn check_turn(
    state: &AppState,
    calls: &Captures,
    id: &str,
    files: &[FileFixture],
    ids: &[String],
    text: &str,
    resumed: bool,
    images: bool,
) {
    let request = serde_json::from_value::<engine::TurnRequest>(json!({
        "conversation_id":id,"text":text,"attachment_ids":ids
    }))
    .unwrap();
    let row = Box::pin(engine::begin_turn(
        &state.db,
        OWNER,
        &request,
        &state.encryption_keys,
    ))
    .await
    .unwrap();
    let mut credential =
        credentials::load_for_conversation(&state.db, &state.encryption_keys, OWNER, id)
            .await
            .unwrap()
            .unwrap();
    let (sender, _receiver) = broadcast::channel(256);
    let mut events = Events {
        checkpoint: None,
        sender,
        cursor: 0,
        reset_notified: false,
    };
    let mut context = None;
    let result = Box::pin(execute_turn(
        state,
        &test_auth_user(OWNER),
        &row,
        text,
        &mut credential,
        Some(SERVER_TURN_POLICY),
        &mut events,
        "block",
        &mut String::new(),
        &mut context,
    ))
    .await
    .unwrap();
    assert!(result.error.is_none());
    let body = calls.lock().await.last().unwrap().body.clone();
    assert_eq!(body.get("conversation").is_some(), resumed);
    let image_count = files
        .iter()
        .filter(|file| file.mime.starts_with("image/"))
        .count();
    let input = if images && image_count > 0 {
        let content = body["input"][0]["content"].as_array().unwrap();
        assert_eq!(content.len(), image_count + 1);
        assert_eq!(content[0]["type"], "input_text");
        for (part, (_, id)) in content[1..].iter().zip(
            files
                .iter()
                .zip(ids)
                .filter(|(file, _)| file.mime.starts_with("image/")),
        ) {
            assert_eq!(part["type"], "input_image");
            assert!(
                part["image_url"]
                    .as_str()
                    .unwrap()
                    .ends_with(&format!("/assistant-attachments/{id}/content"))
            );
        }
        content[0]["text"].as_str().unwrap()
    } else {
        body["input"].as_str().unwrap()
    };
    let marker = context.as_ref().unwrap().binding.marker.as_deref().unwrap();
    assert!(input.starts_with(&format!("[NYXID_CONTEXT:{marker}]")));
    assert_eq!(input.matches("Message attachments (").count(), 1);
    let listing = input
        .lines()
        .find(|line| line.starts_with("Message attachments ("))
        .unwrap()
        .split_once("): ")
        .unwrap()
        .1;
    let listing: Value = serde_json::from_str(listing).unwrap();
    assert_eq!(listing.as_array().unwrap().len(), files.len());
    assert!(input.contains("nyx__attachment_read"));
    for ((file, id), metadata) in files.iter().zip(ids).zip(listing.as_array().unwrap()) {
        assert_eq!(metadata["id"], *id);
        assert_eq!(metadata["name"], file.name);
        assert_eq!(metadata["type"], file.mime);
        assert_eq!(metadata["size"], file.bytes.len());
        assert!(!body["instructions"].as_str().unwrap().contains(id));
        if let Some(expected) = &file.text {
            let mut offset = 0;
            let mut extracted = String::new();
            let mut pages = std::collections::BTreeSet::new();
            loop {
                let page = Box::pin(read_document(state, &credential, id, offset)).await;
                assert_eq!(page["untrusted"], true);
                assert_eq!(page["offset"], offset);
                if let Some(label) = page["section"]["label"].as_str() {
                    pages.insert(label.to_owned());
                }
                extracted.push_str(page["text"].as_str().unwrap());
                let Some(next) = page["next_offset"].as_u64() else {
                    break;
                };
                assert!(next > offset as u64);
                offset = next as usize;
            }
            if file.mime == "application/pdf" {
                assert!(extracted.contains(expected));
                assert!(extracted.contains("Second PDF page"));
                assert!(pages.contains("Page 1"));
                // Page offsets are character offsets in the extracted document.
                let second = extracted.find("Second PDF page").unwrap();
                let second = Box::pin(read_document(state, &credential, id, second)).await;
                assert_eq!(second["section"]["label"], "Page 2");
            } else {
                assert_eq!(&extracted, expected, "{}", file.name);
                if expected.chars().count() > 6000 {
                    assert!(offset > 0);
                }
            }
        } else {
            Box::pin(read_image(state, &credential, id, file)).await;
            if !images {
                assert_eq!(input.matches("Unviewable attachment IDs:").count(), 1);
                assert!(input.contains(crate::services::assistant_upload_service::IMAGE_FALLBACK));
                assert!(input.contains("nyx__machine_save_attachment"));
                assert!(input.contains(id));
            }
        }
    }
    let messages = engine::messages(&state.db, OWNER, id, 20, None)
        .await
        .unwrap();
    let user = messages.iter().rev().find(|m| m.role == "user").unwrap();
    assert_eq!(user.text, text);
    assert!(!user.text.contains(marker));
    Box::pin(engine::finish_turn_with_instructions(
        &state.db,
        &row,
        &credential.api_key_id,
        &Uuid::new_v4().to_string(),
        &result,
        context.as_ref().map(|context| &context.binding),
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn all_ten_upload_types_reach_first_resumed_and_attachment_only_turns() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let (state, calls, server) = setup_options(None, Duration::ZERO, Vec::new(), true).await;
    for file in fixtures() {
        let id = Box::pin(draft_id(&state)).await;
        for (text, resumed) in [
            ("Read my file", false),
            ("Read the next file", true),
            ("", true),
        ] {
            let attachment =
                Box::pin(upload_file(&state, &id, &file, "application/octet-stream")).await;
            Box::pin(check_turn(
                &state,
                &calls,
                &id,
                std::slice::from_ref(&file),
                &[attachment],
                text,
                resumed,
                true,
            ))
            .await;
        }
    }
    assert_eq!(calls.lock().await.len(), 30);
    server.abort();
}

#[tokio::test]
async fn mixed_upload_batch_and_large_document_paging_work_with_the_turn_key() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let (state, calls, server) = setup_options(None, Duration::ZERO, Vec::new(), true).await;
    let id = Box::pin(draft_id(&state)).await;
    let files: Vec<_> = fixtures()
        .into_iter()
        .filter(|file| {
            matches!(
                file.name,
                "photo.png" | "report.pdf" | "data.json" | "table.csv" | "report.docx"
            )
        })
        .collect();
    let mut ids = Vec::new();
    for file in &files {
        ids.push(Box::pin(upload_file(&state, &id, file, "")).await);
    }
    assert_eq!(ids.len(), 5);
    Box::pin(check_turn(
        &state,
        &calls,
        &id,
        &files,
        &ids,
        "Read these together",
        false,
        true,
    ))
    .await;
    let text = "Large UTF-8 document 😀\n".repeat(800);
    let file = FileFixture {
        name: "large.txt",
        mime: "text/plain",
        bytes: text.as_bytes().to_vec(),
        text: Some(text),
    };
    let attachment = Box::pin(upload_file(&state, &id, &file, "text/plain")).await;
    Box::pin(check_turn(
        &state,
        &calls,
        &id,
        &[file],
        &[attachment],
        "Read every page",
        true,
        true,
    ))
    .await;
    server.abort();
}

#[tokio::test]
async fn all_image_types_without_capability_have_model_visible_fallback() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let (state, calls, server) = setup(None, Duration::ZERO).await;
    for file in fixtures()
        .into_iter()
        .filter(|file| file.mime.starts_with("image/"))
    {
        let id = Box::pin(draft_id(&state)).await;
        for resumed in [false, true] {
            let attachment =
                Box::pin(upload_file(&state, &id, &file, "application/octet-stream")).await;
            Box::pin(check_turn(
                &state,
                &calls,
                &id,
                std::slice::from_ref(&file),
                &[attachment],
                "",
                resumed,
                false,
            ))
            .await;
        }
    }
    assert_eq!(calls.lock().await.len(), 8);
    server.abort();
}

#[tokio::test]
async fn unsupported_upload_returns_a_user_visible_error_without_storing_a_file() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    use axum::response::IntoResponse;
    let (state, _, server) = setup(None, Duration::ZERO).await;
    let id = Box::pin(draft_id(&state)).await;
    let request = Request::builder()
        .method("POST")
        .header("x-attachment-name", "drawing.svg")
        .header("content-type", "image/png")
        .body(Body::from("<svg xmlns=\"http://www.w3.org/2000/svg\"/>"))
        .unwrap();
    let error = Box::pin(super::super::super::assistant_uploads::upload(
        State(state.clone()),
        test_auth_user(OWNER),
        axum::extract::Path(id),
        request,
    ))
    .await
    .err()
    .unwrap();
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = axum::body::to_bytes(response.into_body(), 10_000)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        body["message"],
        "Bad request: Malformed or unsupported attachment. Use PDF, DOCX, UTF-8 text, PNG, JPEG, GIF or WebP."
    );
    assert_eq!(
        state
            .db
            .collection::<mongodb::bson::Document>(
                crate::models::assistant_attachment::COLLECTION_NAME
            )
            .count_documents(mongodb::bson::doc! {})
            .await
            .unwrap(),
        0
    );
    server.abort();
}

#[tokio::test]
async fn upload_classification_uses_bytes_and_case_insensitive_extensions_not_browser_mime() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let (state, calls, server) = setup(None, Duration::ZERO).await;
    let id = Box::pin(draft_id(&state)).await;
    let mut resumed = false;
    for (name, browser_mime, fixture_name) in [
        ("data.JSON", "", "data.json"),
        ("data.json", "application/octet-stream", "data.json"),
        ("report.PDF", "text/plain", "report.pdf"),
        ("notes.markdown", "", "notes.md"),
        ("notes.md", "application/octet-stream", "notes.md"),
        ("table.csv", "", "table.csv"),
        ("table.csv", "application/octet-stream", "table.csv"),
        ("table.csv", "application/vnd.ms-excel", "table.csv"),
        ("notes.text", "application/octet-stream", "notes.txt"),
        ("README", "", "notes.txt"),
    ] {
        let mut file = fixtures()
            .into_iter()
            .find(|file| file.name == fixture_name)
            .unwrap();
        file.name = name;
        let attachment = Box::pin(upload_file(&state, &id, &file, browser_mime)).await;
        Box::pin(check_turn(
            &state,
            &calls,
            &id,
            &[file],
            &[attachment],
            "Read this file",
            resumed,
            false,
        ))
        .await;
        resumed = true;
    }
    server.abort();
}

#[tokio::test]
async fn scanned_pdf_runtime_delivers_parent_pages_through_real_thread_image_route() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    use tower::ServiceExt;
    for (channel, replace_credential) in [(false, false), (true, false), (false, true)] {
        let (state, calls, server) = setup_options_with_tools(
            replace_credential.then_some((401, "agent_key_required")),
            Duration::ZERO,
            Vec::new(),
            true,
            0,
            Arc::new(AtomicUsize::new(0)),
        )
        .await;
        crate::services::assistant_upload_service::pdf_vision::ensure_indexes(&state.db)
            .await
            .unwrap();
        state.db.collection::<bson::Document>("feature_flag_overrides").insert_one(doc!{"_id":Uuid::new_v4().to_string(),"flag_key":"assistant:pdf-vision","enabled":true,"target_kind":"global","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),"updated_by":OWNER}).await.unwrap();
        let id = crate::handlers::assistant_uploads::draft(
            State(state.clone()),
            test_auth_user(OWNER),
            Json(crate::handlers::assistant_uploads::Draft { agent_id: None }),
        )
        .await
        .unwrap()
        .0["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let bytes = crate::services::attachment_extraction::pdf_fixture(&[None, None], 0);
        let upload = if channel {
            crate::services::assistant_upload_service::upload_channel_authorized(
                &state.db,
                &state.encryption_keys,
                OWNER,
                &id,
                "scan.pdf",
                bytes,
                crate::models::assistant_channel_media::UploadSource {
                    guest: None,
                    turn_id: String::new(),
                    event_key: "lark-fixture".into(),
                    index: 0,
                    embedded: false,
                    declared: 1,
                },
            )
            .await
            .unwrap()
        } else {
            crate::services::assistant_upload_service::upload(
                &state.db,
                &state.encryption_keys,
                OWNER,
                &id,
                "scan.pdf",
                bytes,
            )
            .await
            .unwrap()
        };
        let image = fixtures()
            .into_iter()
            .find(|file| file.name == "photo.png")
            .unwrap();
        let image_id = Box::pin(upload_file(&state, &id, &image, image.mime)).await;
        let second = crate::services::assistant_upload_service::upload(
            &state.db,
            &state.encryption_keys,
            OWNER,
            &id,
            "second.pdf",
            crate::services::attachment_extraction::pdf_fixture(
                &[
                    None,
                    Some("Second page text"),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                ],
                0,
            ),
        )
        .await
        .unwrap();
        let request = serde_json::from_value::<engine::TurnRequest>(
            json!({"conversation_id":id,"text":"Read every page","attachment_ids":[image_id,upload.id,second.id]}),
        )
        .unwrap();
        let row = Box::pin(engine::begin_turn(
            &state.db,
            OWNER,
            &request,
            &state.encryption_keys,
        ))
        .await
        .unwrap();
        let mut credential =
            credentials::load_for_conversation(&state.db, &state.encryption_keys, OWNER, &id)
                .await
                .unwrap()
                .unwrap();
        let (sender, _receiver) = broadcast::channel(256);
        let mut events = Events {
            checkpoint: None,
            sender,
            cursor: 0,
            reset_notified: false,
        };
        let result = Box::pin(execute_turn(
            &state,
            &test_auth_user(OWNER),
            &row,
            "Read every page",
            &mut credential,
            Some(SERVER_TURN_POLICY),
            &mut events,
            "block",
            &mut String::new(),
            &mut None,
        ))
        .await
        .unwrap();
        assert!(result.error.is_none(), "{:?}", result.error);
        if replace_credential {
            let captures = calls.lock().await;
            assert_eq!(captures.len(), 2);
            let first = captures[0].body["input"][0]["content"].as_array().unwrap();
            assert_eq!(
                first
                    .iter()
                    .filter(|part| part["type"] == "input_image")
                    .count(),
                8
            );
            let retry = captures[1].body["input"][0]["content"].as_array().unwrap();
            assert_eq!(
                retry
                    .iter()
                    .filter(|part| part["type"] == "input_image")
                    .count(),
                1
            );
            assert!(
                retry[0]["text"]
                    .as_str()
                    .unwrap()
                    .contains("credential_replaced")
            );
            let saved = crate::services::assistant_upload_service::turn_attachments(
                &state.db,
                OWNER,
                &id,
                &row.active_turn.as_ref().unwrap().turn_id,
            )
            .await
            .unwrap();
            let report = saved
                .iter()
                .find(|item| item.id == upload.id)
                .unwrap()
                .pdf
                .as_ref()
                .unwrap();
            assert!(report.image_pages.is_empty());
            assert_eq!(report.unread_pages, [1, 2]);
            assert_eq!(report.reason.as_deref(), Some("credential_replaced"));
            server.abort();
            state.db.drop().await.unwrap();
            continue;
        }
        let body = calls.lock().await.last().unwrap().body.clone();
        let parts = body["input"][0]["content"].as_array().unwrap();
        assert_eq!(parts.len(), 16); // request text, ordinary image, seven labeled PDF images
        assert_eq!(parts[1]["type"], "input_image");
        assert!(
            parts[0]["text"]
                .as_str()
                .unwrap()
                .contains("PDF page coverage")
        );
        let router = Router::new()
            .route(
                "/api/v1/assistant-attachments/{id}/content",
                axum::routing::get(super::super::super::assistant_uploads::thread_image),
            )
            .with_state(state.clone());
        let expected = [
            (&upload.id, 1),
            (&upload.id, 2),
            (&second.id, 1),
            (&second.id, 3),
            (&second.id, 4),
            (&second.id, 5),
            (&second.id, 6),
        ];
        for (offset, (parent, page)) in expected.into_iter().enumerate() {
            let label = &parts[2 + offset * 2];
            assert_eq!(label["type"], "input_text");
            let label: Value =
                serde_json::from_str(label["text"].as_str().unwrap().split_once(": ").unwrap().1)
                    .unwrap();
            let image = &parts[3 + offset * 2];
            assert_eq!(image["type"], "input_image");
            assert_eq!(label["image_index"], offset + 2);
            assert_eq!(label["parent_attachment_id"], *parent);
            assert_eq!(label["page"], page);
            assert!(image["image_url"].as_str().unwrap().ends_with(&format!(
                "/{}/content",
                label["source_attachment_id"].as_str().unwrap()
            )));
        }
        for part in parts.iter().filter(|part| part["type"] == "input_image") {
            let url = url::Url::parse(part["image_url"].as_str().unwrap()).unwrap();
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(url.path())
                        .header("x-api-key", credential.raw_key.as_str())
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let expected_mime = if url.path().ends_with(&format!("/{image_id}/content")) {
                "image/png"
            } else {
                "image/jpeg"
            };
            assert_eq!(response.headers()["content-type"], expected_mime);
            let bytes = axum::body::to_bytes(response.into_body(), 3 * 1024 * 1024)
                .await
                .unwrap();
            assert!(image::load_from_memory(&bytes).is_ok());
        }
        let stored = crate::services::assistant_upload_service::turn_attachments(
            &state.db,
            OWNER,
            &id,
            &row.active_turn.as_ref().unwrap().turn_id,
        )
        .await
        .unwrap();
        assert_eq!(stored.len(), 3); // the ordinary image and two parent PDFs, no derived attachments
        let first = stored.iter().find(|item| item.id == upload.id).unwrap();
        assert_eq!(first.pdf.as_ref().unwrap().image_pages, vec![1, 2]);
        let second_coverage = stored
            .iter()
            .find(|item| item.id == second.id)
            .unwrap()
            .pdf
            .as_ref()
            .unwrap();
        assert_eq!(second_coverage.text_pages, vec![2]);
        assert_eq!(second_coverage.image_pages, vec![1, 3, 4, 5, 6]);
        assert_eq!(second_coverage.unread_pages, vec![7, 8, 9]);
        let dto = serde_json::to_value(AttachmentResponse::from(first.clone())).unwrap();
        assert_eq!(dto["pdf"]["image_pages"], json!([1, 2]));
        let text = read_document(&state, &credential, &upload.id, 0).await;
        assert_eq!(text["error"], "attachment_has_no_extracted_text");
        assert_eq!(text["pdf"]["image_pages"], json!([1, 2]));
        let mixed = read_document(&state, &credential, &second.id, 0).await;
        assert!(mixed["text"].as_str().unwrap().contains("Second page text"));
        assert_eq!(mixed["complete"], true);
        assert_eq!(mixed["text_complete"], true);
        assert_eq!(mixed["next_offset"], Value::Null);
        assert_eq!(mixed["pdf"]["unread_pages"], json!([7, 8, 9]));
        server.abort();
        state.db.drop().await.unwrap();
    }
}

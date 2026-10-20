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
    assert_eq!(rpc["result"]["isError"], false);
    let text = rpc["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.chars().count() < 10_000);
    serde_json::from_str(text).unwrap()
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

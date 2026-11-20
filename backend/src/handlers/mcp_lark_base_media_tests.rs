//! Assert the actual provider wire request, rather than a successful file token.
use super::*;

#[path = "mcp_lark_base_filename_tests.rs"]
mod original_names;

fn raster(format: image::ImageFormat) -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 2)
        .write_to(&mut bytes, format)
        .unwrap();
    bytes.into_inner()
}

fn assert_upload(request: &wiremock::Request, bytes: &[u8], name: &str, kind: &str) {
    let content_type = request.headers["content-type"].to_str().unwrap();
    let boundary = content_type
        .strip_prefix("multipart/form-data; boundary=")
        .unwrap();
    let file_header = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\nContent-Type: {kind}\r\n\r\n"
    );
    let offset = request
        .body
        .windows(file_header.len())
        .position(|part| part == file_header.as_bytes())
        .expect("typed file part");
    let prefix = std::str::from_utf8(&request.body[..offset]).unwrap();
    for (field, value) in [
        ("file_name", name.to_owned()),
        (
            "parent_type",
            if kind.starts_with("image/") {
                "bitable_image"
            } else {
                "bitable_file"
            }
            .into(),
        ),
        ("parent_node", "base".into()),
        ("size", bytes.len().to_string()),
        ("extra", "{\"drive_route_token\":\"base\"}".into()),
    ] {
        assert!(
            prefix.contains(&format!("name=\"{field}\"\r\n\r\n{value}\r\n")),
            "{prefix}"
        );
    }
    let payload = offset + file_header.len();
    assert_eq!(&request.body[payload..payload + bytes.len()], bytes);
    assert_eq!(
        &request.body[payload + bytes.len()..],
        format!("\r\n--{boundary}--\r\n").as_bytes()
    );
}

#[tokio::test]
async fn lark_base_media_retained_images_and_pdfs_transmit_original_bytes_and_typed_defaults() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    for (kind, bytes, source_name, label, expected) in [
        (
            "image/png",
            raster(image::ImageFormat::Png),
            "attachment",
            "attachment",
            "attachment.png",
        ),
        (
            "image/jpeg",
            raster(image::ImageFormat::Jpeg),
            "attachment",
            "attachment",
            "attachment.jpg",
        ),
        (
            "application/pdf",
            pdf(),
            "invoice.pdf",
            "invoice.pdf",
            "invoice.pdf",
        ),
        (
            "image/jpeg",
            raster(image::ImageFormat::Jpeg),
            "receipt",
            "receipt",
            "receipt",
        ),
        (
            "image/jpeg",
            raster(image::ImageFormat::Jpeg),
            "收据.JPG",
            "收据.JPG",
            "收据.JPG",
        ),
        (
            "application/pdf",
            pdf(),
            "Original receipt",
            "Original receipt",
            "Original receipt",
        ),
        (
            "application/pdf",
            pdf(),
            "attachment",
            "attachment",
            "attachment.pdf",
        ),
        (
            "text/plain",
            b"original text".to_vec(),
            "attachment",
            "attachment",
            "attachment.txt",
        ),
        (
            "text/markdown",
            b"# Receipt".to_vec(),
            "source.md",
            "attachment",
            "attachment.md",
        ),
        (
            "text/csv",
            b"amount,currency\n12.34,SGD\n".to_vec(),
            "source.csv",
            "attachment",
            "attachment.csv",
        ),
        (
            "application/json",
            br#"{"amount":12.34}"#.to_vec(),
            "source.json",
            "attachment",
            "attachment.json",
        ),
    ] {
        let u = setup("base_media_retained").await;
        let id = bind_named_upload(&u.f, source_name, &bytes).await;
        // This is the persisted shape of an older channel image: a verified
        // type and encrypted bytes, with a generic extensionless display label.
        let rows =
            u.f.state
                .db
                .collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME);
        rows.update_one(
            doc! {"_id":&id},
            doc! {"$set":{"label":label,
            "source":{"event_key":"retained-channel-source", "declared":1_i64, "index":0_i64,
                "embedded":false,"turn_id":"previous-channel-turn"}}, "$unset":{"filename_is_original":""}},
        )
        .await
        .unwrap();
        let stored = rows.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
        assert_eq!(stored.get_str("label").unwrap(), label);
        assert_eq!(stored.get_str("content_type").unwrap(), kind);
        let auth = authenticate(&u.f).await;
        for meta in [false, true] {
            let response = named_call(&u, &auth, meta, UPLOAD_OPERATION, json!({"attachment_id":id,
                "parent_type":if kind.starts_with("image/") {"bitable_image"} else {"bitable_file"}, "parent_node":"base"})).await;
            assert_eq!(response["result"]["isError"], false, "{response}");
        }
        let response = call(&u, &auth, false, args(&id)).await;
        assert_eq!(payload(&response)["record_id"], "rec_receipt");
        assert_eq!(
            payload(&call(&u, &auth, true, args(&id)).await)["already_created"],
            true
        );
        let sent = requests(&u).await;
        let uploads: Vec<_> = sent
            .iter()
            .filter(|r| r.url.path().ends_with("upload_all"))
            .collect();
        assert_eq!(uploads.len(), 3);
        for upload in uploads {
            assert_upload(upload, &bytes, expected, kind);
        }
        let create = sent
            .iter()
            .find(|r| r.url.path().ends_with("/records"))
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&create.body).unwrap()["fields"]["Receipt"],
            json!([{"file_token":"file_receipt"}])
        );
        use sha2::{Digest, Sha256};
        assert_eq!(
            ledger(&u).await.files[0].content_sha256,
            hex::encode(Sha256::digest(&bytes))
        );
        let after = rows.find_one(doc! {"_id":&id}).await.unwrap().unwrap();
        assert_eq!(after.get("source"), stored.get("source"));
        assert_eq!(after.get("content_sha256"), stored.get("content_sha256"));
        // Repair happens at egress and does not mutate retained source metadata.
        assert_eq!(
            rows.find_one(doc! {"_id":&id})
                .await
                .unwrap()
                .unwrap()
                .get_str("label")
                .unwrap(),
            label
        );
        cleanup(u).await;
    }
}

#[tokio::test]
async fn lark_base_media_long_unicode_defaults_survive_multipart_and_explicit_names_stay_exact() {
    let _budget_guard = crate::services::assistant_upload_service::byte_budget::test_guard().await;
    let u = setup("base_media_names").await;
    let bytes = raster(image::ImageFormat::Png);
    let label = "照片".repeat(120);
    let id = bind_named_upload(&u.f, &label, &bytes).await;
    let auth = authenticate(&u.f).await;
    let default = uploads::safe_name(&label);
    for (meta, explicit) in [
        (false, None),
        (true, Some("owner-chosen")),
        (false, Some("receipt.JPEG")),
    ] {
        let mut input =
            json!({"attachment_id":id,"parent_type":"bitable_image","parent_node":"base"});
        if let Some(name) = explicit {
            input["file_name"] = json!(name);
        }
        let response = named_call(&u, &auth, meta, UPLOAD_OPERATION, input).await;
        assert_eq!(response["result"]["isError"], false, "{response}");
        assert_upload(
            requests(&u).await.last().unwrap(),
            &bytes,
            explicit.unwrap_or(&default),
            "image/png",
        );
    }
    let response = call(&u, &auth, true, args(&id)).await;
    assert_eq!(payload(&response)["record_id"], "rec_receipt");
    let sent = requests(&u).await;
    assert_upload(
        sent.iter()
            .rev()
            .find(|r| r.url.path().ends_with("upload_all"))
            .unwrap(),
        &bytes,
        &default,
        "image/png",
    );
    cleanup(u).await;
}

use super::*;
use base64::Engine;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn png() -> Vec<u8> {
    let mut output = std::io::Cursor::new(Vec::new());
    ::image::DynamicImage::new_rgb8(2, 2)
        .write_to(&mut output, ::image::ImageFormat::Png)
        .unwrap();
    output.into_inner()
}
async fn load(
    args: Value,
    origin: Option<&str>,
    timeout: Duration,
) -> AppResult<(bytes::Bytes, String)> {
    materialize_with(args.to_string().as_bytes(), origin, timeout).await
}

#[tokio::test]
async fn lark_im_image_base64_has_message_multipart_and_validated_image_bytes() {
    let image = png();
    let args = json!({"image_base64":base64::engine::general_purpose::STANDARD.encode(&image),"filename":"祝福.png","mime_type":"image/png"});
    let (body, ct) = load(args, None, TIMEOUT).await.unwrap();
    assert!(ct.starts_with("multipart/form-data; boundary="));
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("name=\"image_type\"\r\n\r\nmessage"));
    assert!(text.contains("name=\"image\"; filename=\"祝福.png\""));
    assert!(text.contains("Content-Type: image/png"));
    assert!(body.windows(image.len()).any(|w| w == image));
}

#[tokio::test]
async fn lark_im_image_url_checks_content_type_actual_bytes_and_redirects() {
    let server = MockServer::start().await;
    for (name, template) in [
        (
            "image",
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .set_body_bytes(png()),
        ),
        (
            "mismatch",
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/jpeg")
                .set_body_bytes(png()),
        ),
        ("missing", ResponseTemplate::new(200).set_body_bytes(png())),
        (
            "html",
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/html")
                .set_body_string("secret html"),
        ),
        (
            "large",
            ResponseTemplate::new(200)
                .insert_header("content-type", "image/png")
                .set_body_bytes(vec![0; MAX_BYTES + 1]),
        ),
        (
            "redirect",
            ResponseTemplate::new(302).insert_header("location", "http://127.0.0.1/private"),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(format!("/{name}")))
            .respond_with(template)
            .mount(&server)
            .await;
        let result = load(
            json!({"image_url":format!("{}/{name}",server.uri())}),
            Some(&server.uri()),
            TIMEOUT,
        )
        .await;
        assert_eq!(result.is_ok(), name == "image", "{name}");
    }
    for request in server.received_requests().await.unwrap() {
        assert!(request.headers.get("authorization").is_none());
        assert!(request.headers.get("cookie").is_none());
    }
}

#[tokio::test]
async fn lark_im_image_timeout_unsafe_urls_non_images_and_oversize_fail_without_upload() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(1)))
        .mount(&server)
        .await;
    let error = load(
        json!({"image_url":server.uri()}),
        Some(&server.uri()),
        Duration::from_millis(20),
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("lark_image_fetch_timeout"));
    for url in [
        "http://127.0.0.1/secret",
        "https://127.0.0.1/secret",
        "https://169.254.169.254/latest/meta-data",
        "file:///etc/passwd",
        "https://user:secret@example.com/image",
    ] {
        assert!(load(json!({"image_url":url}), None, TIMEOUT).await.is_err());
    }
    for args in [
        json!({}),
        json!({"image_base64":"YQ==","image_url":"https://example.com/image"}),
        json!({"image_base64":"not-base64"}),
        json!({"image_base64":base64::engine::general_purpose::STANDARD.encode(b"not an image")}),
        json!({"image_base64":base64::engine::general_purpose::STANDARD.encode(png()),"mime_type":"image/jpeg"}),
        json!({"image_base64":base64::engine::general_purpose::STANDARD.encode(vec![0;MAX_BYTES+1])}),
        json!({"image_base64":"YQ==","filename":"a\r\nInjected"}),
    ] {
        assert!(load(args, None, TIMEOUT).await.is_err());
    }
}

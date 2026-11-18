use super::*;
use crate::services::{lark_attachment_tool as attachment, mcp_service as mcp};

pub(crate) fn compiled(slug: &str, name: &str) -> (DownstreamService, McpToolEndpoint) {
    let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
    catalog.slug = slug.into();
    catalog.created_by = "system".into();
    let input = catalog_spec_sync::destination_endpoint_inputs(
        &catalog,
        &catalog_spec_registry::spec_for_slug(slug).unwrap(),
    )
    .unwrap()
    .into_iter()
    .find(|e| e.name == name)
    .unwrap();
    let mut endpoint = McpToolEndpoint {
        endpoint_id: "endpoint".into(),
        name: input.name,
        method: input.method,
        path: input.path,
        target_id: input.target_id,
        parameters: input.parameters,
        request_body_schema: input.request_body_schema,
        request_content_type: input.request_content_type,
        request_body_required: input.request_body_required,
        response: input.response,
        ..Default::default()
    };
    endpoint.attachment_input = attachment::stored_projection(Some(&catalog), &endpoint);
    (catalog, endpoint)
}

#[test]
fn lark_im_text_serialization_preserves_unicode_escapes_and_split_parts() {
    for text in [
        "第一行\n第二行 🎉",
        "quotes: \"a\"; path: C:\\folder",
        "real\nnewline",
        r"literal\nnewline",
    ] {
        for part in [
            text.to_owned(),
            format!("Part 1: {text}"),
            format!("Part 2: {text}"),
        ] {
            let mut body =
                json!({"msg_type":"text","text":part,"receive_id":"chat","uuid":"stable"});
            normalize_message(&mut body).unwrap();
            let content: Value = serde_json::from_str(body["content"].as_str().unwrap()).unwrap();
            assert_eq!(content, json!({"text":part}));
            assert!(body.get("text").is_none());
            assert_eq!(body["uuid"], "stable");
            let wire = body.to_string();
            let read: Value = serde_json::from_str(&wire).unwrap();
            assert_eq!(read, body);
        }
    }
}

#[test]
fn lark_im_raw_strings_are_byte_identical_and_objects_serialize_once() {
    let raw = "{ \"text\": \"你好\\n\\\"hello\\\"\\\\path\" }";
    let original = json!({"msg_type":"text","content":raw,"receive_id":"chat"});
    let mut body = original.clone();
    normalize_message(&mut body).unwrap();
    assert_eq!(body.to_string().as_bytes(), original.to_string().as_bytes());
    for msg_type in ["post", "interactive", "image"] {
        let object = json!({"nested":{"text":"中文\n🙂"}});
        let mut body = json!({"msg_type":msg_type,"content":object});
        normalize_message(&mut body).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(body["content"].as_str().unwrap()).unwrap(),
            object
        );
    }
}

#[test]
fn lark_im_invalid_content_is_bounded_and_diagnostics_do_not_echo() {
    for mut body in [
        json!({"msg_type":"text"}),
        json!({"msg_type":"text","text":"secret","content":{}}),
        json!({"msg_type":"image","text":"secret"}),
        json!({"msg_type":"text","content":"secret"}),
        json!({"msg_type":"text","content":"[]"}),
        json!({"msg_type":"text","content":"\"double encoded\""}),
        json!({"msg_type":"text","content":{}}),
    ] {
        let error = normalize_message(&mut body).err().unwrap().to_string();
        assert!(error.contains("lark_message_content_invalid"));
        assert!(!error.contains("secret"));
    }
    for mode in [
        json!({"msg_type":"text","text":"secret"}),
        json!({"msg_type":"text","content":{"text":"secret"}}),
    ] {
        let mode = normalize_message(&mut mode.clone()).unwrap();
        let err = response(
            Kind::Message,
            Some(&mode),
            200,
            r#"{"code":230001,"msg":"secret"}"#,
        )
        .unwrap_err()
        .to_string();
        assert!(!err.contains("secret"));
        for field in [
            "msg_type",
            "mode",
            "local_json_parsed",
            "request_sent",
            "230001",
        ] {
            assert!(err.contains(field));
        }
    }
}

#[test]
fn lark_im_catalog_contracts_all_four_slugs_are_trusted_but_mounted_or_edited_specs_are_not() {
    for slug in ["api-lark", "api-feishu", "api-lark-bot", "api-feishu-bot"] {
        for operation in ["im_message_create", "im_message_reply", "im_image_create"] {
            let (catalog, endpoint) = compiled(slug, operation);
            let expected = if operation == "im_image_create" {
                Kind::Image
            } else {
                Kind::Message
            };
            assert_eq!(kind(&endpoint), Some(expected), "{slug}/{operation}");
            let schema = mcp::build_input_schema(&endpoint);
            assert!(
                schema["properties"]
                    .get(if expected == Kind::Image {
                        "image_url"
                    } else {
                        "text"
                    })
                    .is_some()
            );
            assert!(stored(None, &endpoint).is_none());
            let mut edited = endpoint.clone();
            edited.path.push_str("/other");
            assert!(stored(Some(&catalog), &edited).is_none());
            edited = endpoint.clone();
            edited.request_body_schema.as_mut().unwrap()["additionalProperties"] = json!(true);
            assert!(stored(Some(&catalog), &edited).is_none());
            let mut non_system = catalog.clone();
            non_system.created_by = "admin".into();
            assert!(stored(Some(&non_system), &endpoint).is_none());
            if expected == Kind::Message {
                let mut legacy = endpoint.clone();
                legacy_message_contract(&mut legacy);
                assert_eq!(
                    legacy.request_body_schema.as_ref().unwrap()["properties"]["content"]["type"],
                    "string"
                );
                assert!(
                    legacy.request_body_schema.as_ref().unwrap()["properties"]
                        .get("text")
                        .is_none()
                );
            } else {
                assert!(matches!(
                    attachment::untrusted(&endpoint),
                    Some(attachment::Projection::ImRefused)
                ));
                let marks = catalog_spec_registry::operation_marks(
                    slug,
                    &endpoint.method,
                    &endpoint.path,
                    &endpoint.name,
                );
                assert_eq!(marks.changes_existing, Some(false));
                assert!(!marks.destructive);
            }
        }
    }
}

#[test]
fn lark_im_image_response_requires_success_and_returns_only_key() {
    assert_eq!(
        response(
            Kind::Image,
            None,
            200,
            r#"{"code":0,"data":{"image_key":"img_test","secret":"hidden"}}"#
        )
        .unwrap(),
        r#"{"image_key":"img_test"}"#
    );
    for (status, body) in [
        (200, r#"{"code":1,"msg":"secret"}"#),
        (403, "secret"),
        (200, r#"{"code":0,"data":{}}"#),
        (502, "secret"),
    ] {
        let error = response(Kind::Image, None, status, body)
            .unwrap_err()
            .to_string();
        assert!(error.contains("lark_image_upload_rejected"));
        assert!(!error.contains("secret"));
    }
}

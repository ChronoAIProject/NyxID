use super::*;

#[test]
fn image_publication_customer_operation_names_and_paths_do_not_grant_or_intercept_capabilities() {
    let spec = super::super::catalog_spec_registry::spec_for_slug("api-linkedin").unwrap();
    let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
    catalog.slug = "customer-service".into();
    catalog.created_by = "customer".into();
    let input = super::super::catalog_spec_sync::destination_endpoint_inputs(&catalog, &spec)
        .unwrap()
        .into_iter()
        .find(|i| i.name == OPERATION)
        .unwrap();
    let row = super::super::service_endpoint_service::endpoint_from_input(
        &catalog.id,
        input,
        PublicationState::Published,
    );
    let mut endpoints = mcp::service_endpoints_to_mcp(&[&row]);
    assert!(!candidate(&endpoints[0]));
    assert!(mcp::build_input_schema(&endpoints[0])["properties"]["action"].is_object());
    mark_catalog_operations(Some(&catalog), &mut endpoints);
    append_catalog_operation(&mut endpoints, &[&row], Some(&catalog));
    assert_eq!(endpoints.len(), 1);
    assert!(!candidate(&endpoints[0]));
    // A remote spec on LinkedIn with a colliding operation name keeps its own
    // contract; it is never silently replaced by a privileged upload operation.
    catalog.slug = "api-linkedin".into();
    catalog.created_by = "system".into();
    append_catalog_operation(&mut endpoints, &[&row], Some(&catalog));
    assert_eq!(endpoints.len(), 1);
    assert!(!candidate(&endpoints[0]));
}

#[test]
fn image_publication_failure_codes_cannot_retain_error_details() {
    let secret = "https://provider.test/signed?token=never-store-this";
    for error in [
        AppError::Forbidden(secret.into()),
        AppError::Internal(secret.into()),
        AppError::Forbidden(format!("image_publication_{secret}")),
    ] {
        assert_eq!(failure_code(&error), "provider_or_authority_refused");
    }
}

#[test]
fn image_publication_refusals_and_ambiguous_effects_have_distinct_status() {
    for status in [400, 401, 403, 404, 405, 406, 410, 413, 415, 422, 429] {
        assert!(definite_refusal(status));
        let outcome = json!({"state":"failed","http_status":status});
        let result = result_status("publish", &outcome, true, Some(status));
        assert_eq!((result.tool, result.audit), (status, status));
    }
    for status in [408, 409, 500, 502, 503, 504] {
        assert!(!definite_refusal(status));
        let outcome = json!({"state":"unknown","http_status":status});
        let result = result_status("publish", &outcome, true, Some(status));
        assert_eq!((result.tool, result.audit), (409, status));
    }
    let created = json!({"state":"created","http_status":201,"verification":"readback_permission_unavailable"});
    let result = result_status("publish", &created, true, Some(403));
    assert_eq!((result.tool, result.audit), (201, 201));
    let read = result_status("status", &created, true, Some(403));
    assert_eq!((read.tool, read.audit), (200, 403));
    let replay = result_status("publish", &created, false, None);
    assert_eq!(replay.tool, 200);
    let not_sent = result_status(
        "publish",
        &json!({"state":"ready","code":"not_sent"}),
        false,
        None,
    );
    assert_eq!(not_sent.tool, 409);
}

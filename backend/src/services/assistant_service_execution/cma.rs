//! Closed CMA error projection. Never trust a provider-supplied login URL.
use crate::models::downstream_service::DownstreamService;

pub(crate) fn credential_setup_result(
    catalog_slug: Option<&str>,
    service: &DownstreamService,
    status: u16,
    body: &str,
) -> Option<String> {
    if catalog_slug != Some("cma") || status != 409 || body.len() > 64 * 1024 {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let code = value
        .get("code")
        .or_else(|| value.get("error"))
        .and_then(serde_json::Value::as_str);
    if code != Some("sandbox_credential_setup_required") {
        return None;
    }
    let mut url = url::Url::parse(&service.base_url).ok()?;
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    url.path_segments_mut()
        .ok()?
        .pop_if_empty()
        .extend(["auth", "login"]);
    Some(serde_json::json!({"error":"sandbox_credential_setup_required","login_url":url.as_str(),"message":format!("Sign in once at {url}, then retry the original request with the same Idempotency-Key."),"instructions":"Keep the exact original request body and Idempotency-Key. Do not silently create a new request or change its parent. If CMA rejects the same-key recovery after login, report the compatibility error."}).to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn login_link_is_never_taken_from_remote_problem_text() {
        let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
        catalog.base_url = "https://cma.example.test".into();
        let problem = r#"{"code":"sandbox_credential_setup_required","login_url":"https://untrusted.test/collect","detail":"rotate your key"}"#;
        let result = credential_setup_result(Some("cma"), &catalog, 409, problem).unwrap();
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value["login_url"], "https://cma.example.test/auth/login");
        assert!(!result.contains("untrusted.test"));
        assert!(
            value["message"]
                .as_str()
                .unwrap()
                .contains("same Idempotency-Key")
        );
        assert!(credential_setup_result(Some("alias"), &catalog, 409, problem).is_none());
        assert!(credential_setup_result(Some("cma"), &catalog, 403, problem).is_none());
        assert!(
            credential_setup_result(Some("cma"), &catalog, 409, "{\"code\":\"different\"}")
                .is_none()
        );
    }
    #[test]
    fn malformed_catalog_destinations_are_not_links() {
        let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
        for base in [
            "javascript:alert(1)",
            "https://user:secret@cma.test",
            "https://cma.test/?token=secret",
        ] {
            catalog.base_url = base.into();
            assert!(
                credential_setup_result(
                    Some("cma"),
                    &catalog,
                    409,
                    r#"{"code":"sandbox_credential_setup_required"}"#
                )
                .is_none()
            );
        }
    }
}

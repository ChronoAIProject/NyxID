use crate::errors::{AppError, AppResult};

/// Keep the social-login scheme contract unchanged. Service OAuth applies
/// additional URL constraints before persisting a return destination.
pub(crate) fn is_supported_mobile_redirect_uri(uri: &str) -> bool {
    uri.starts_with("nyxid://") || uri.starts_with("exp://")
}

pub(crate) fn validate_app_return_uri(uri: &str, configured_schemes: &[String]) -> AppResult<()> {
    let Some((scheme, _)) = uri.split_once("://") else {
        return Err(AppError::ValidationError("Invalid app_return_uri".into()));
    };
    let valid_scheme = scheme == "nyxid"
        || configured_schemes
            .iter()
            .any(|configured| configured == scheme);
    let valid = uri.len() <= 512
        && !uri.chars().any(|c| c.is_control() || c.is_whitespace())
        && valid_scheme
        && uri == format!("{scheme}://services/oauth-return")
        && url::Url::parse(uri).is_ok();
    if !valid {
        return Err(AppError::ValidationError("Invalid app_return_uri".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_return_validation_matrix() {
        assert!(validate_app_return_uri("nyxid://services/oauth-return", &[]).is_ok());
        for uri in [
            "",
            "https://example.com",
            "https://evil.example",
            "http://evil.example",
            "javascript:alert(1)",
            "data:text/plain,x",
            "file:///tmp/x",
            "nyxidx://services/oauth-return",
            "nyxid:/x",
            "nyxid://",
            "nyxid://[",
            "nyxid://user@services/oauth-return",
            "nyxid://@services/oauth-return",
            "nyxid://user:password@services/oauth-return",
            "nyxid://services/oauth-return/",
            "nyxid://services/oauth-return?x=1",
            "nyxid://services/oauth-return#fragment",
            "nyxid://services/other",
            "nyxid://activity",
            "nyxid://services/\nreturn",
            "nyxid://services/\treturn",
            "nyxid://services/\0return",
            " nyxid://services/oauth-return",
            "nyxid://services/oauth return",
            "/services/oauth-return",
            "NYXID://services/oauth-return",
            "exp://127.0.0.1:8081/--/services/oauth-return",
        ] {
            assert!(validate_app_return_uri(uri, &[]).is_err(), "{uri:?}");
        }
        let oversized = format!("nyxid://services/oauth-return{}", "x".repeat(512));
        assert!(validate_app_return_uri(&oversized, &[]).is_err());
        assert!(validate_app_return_uri("acme-nyx://services/oauth-return", &[]).is_err());
        assert!(
            validate_app_return_uri("acme-nyx://services/oauth-return", &["acme-nyx".into()])
                .is_ok()
        );
        assert!(
            validate_app_return_uri("ACME-NYX://services/oauth-return", &["acme-nyx".into()])
                .is_err()
        );
    }
}

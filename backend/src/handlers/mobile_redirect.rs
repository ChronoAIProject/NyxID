use crate::errors::{AppError, AppResult};

/// Keep the social-login scheme contract unchanged. Service OAuth applies
/// additional URL constraints before persisting a return destination.
pub(crate) fn is_supported_mobile_redirect_uri(uri: &str) -> bool {
    uri.starts_with("nyxid://") || uri.starts_with("exp://")
}

pub(crate) fn validate_app_return_uri(uri: &str, production: bool) -> AppResult<()> {
    let valid = uri.len() <= 512
        && !uri.chars().any(|c| c.is_control() || c.is_whitespace())
        && is_supported_mobile_redirect_uri(uri)
        && (!production || !uri.starts_with("exp://"))
        && uri.split_once("://").is_some_and(|(_, rest)| {
            !rest
                .split(['/', '?', '#'])
                .next()
                .unwrap_or("")
                .contains('@')
        })
        && url::Url::parse(uri).is_ok_and(|url| {
            url.host_str().is_some()
                && url.fragment().is_none()
                && url.username().is_empty()
                && url.password().is_none()
        });
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
        for uri in [
            "nyxid://services/oauth-return",
            "exp://127.0.0.1:8081/--/services/oauth-return",
            "nyxid://services/oauth-return?screen=connection",
        ] {
            assert!(validate_app_return_uri(uri, false).is_ok(), "{uri}");
        }
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
            "nyxid://services/oauth-return#fragment",
            "nyxid://services/\nreturn",
            "nyxid://services/\treturn",
            "nyxid://services/\0return",
            " nyxid://services/oauth-return",
            "nyxid://services/oauth return",
            "/services/oauth-return",
            "NYXID://services/oauth-return",
        ] {
            assert!(validate_app_return_uri(uri, false).is_err(), "{uri:?}");
        }
        assert!(validate_app_return_uri("exp://127.0.0.1:8081", true).is_err());
        assert!(validate_app_return_uri("nyxid://services/oauth-return", true).is_ok());
        let boundary = format!("nyxid://services/{}", "a".repeat(495));
        assert_eq!(boundary.len(), 512);
        assert!(validate_app_return_uri(&boundary, false).is_ok());
        assert!(validate_app_return_uri(&(boundary + "a"), false).is_err());
    }
}

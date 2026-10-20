//! Catalog-owned vendor authorization schemes; user connections cannot override them.
use crate::errors::{AppError, AppResult};

/// RFC 9110 token, bounded to one short authorization scheme.
pub fn validate(auth_method: &str, scheme: Option<&str>) -> AppResult<()> {
    if let Some(scheme) = scheme
        && (auth_method != "bearer"
            || scheme.is_empty()
            || scheme.len() > 64
            || !scheme
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)))
    {
        return Err(AppError::ValidationError(
            "auth_scheme requires bearer auth and an RFC 9110 token of 1–64 bytes".into(),
        ));
    }
    Ok(())
}

/// All server-side catalog bearer injection shares this value builder. Absence
/// preserves the exact existing Bearer wire value. Never log the returned value.
pub fn header_value(scheme: Option<&str>, credential: &str) -> AppResult<String> {
    validate("bearer", scheme)?;
    Ok(format!("{} {credential}", scheme.unwrap_or("Bearer")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_auth_scheme_is_bounded_and_preserves_default_wire_bytes() {
        assert_eq!(
            header_value(None, "local-token").unwrap(),
            "Bearer local-token"
        );
        assert_eq!(
            header_value(Some("Zoho-oauthtoken"), "local-token").unwrap(),
            "Zoho-oauthtoken local-token"
        );
        for scheme in [
            "",
            " Bearer",
            "Bearer ",
            "two words",
            "Bearer\r\n",
            "é",
            &"a".repeat(65),
        ] {
            assert!(validate("bearer", Some(scheme)).is_err());
        }
        assert!(validate("bearer", Some(&"a".repeat(64))).is_ok());
        assert!(validate("bearer", Some("!#$%&'*+-.^_`|~")).is_ok());
        assert!(validate("header", Some("Bearer")).is_err());
        assert!(validate("none", None).is_ok());
    }
}

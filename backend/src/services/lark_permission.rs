//! Shared helper for building Lark / Feishu permission setup URLs.
//!
//! Feishu's open platform serves a deep link of the shape
//! `https://open.feishu.cn/app/{app_id}/auth?q={scope1,scope2,...}&op_from=openapi`
//! that lands the developer on the app's "Permissions & Scopes" page with
//! every scope in `q` pre-checked, ready for "批量开通" (Bulk Enable).
//!
//! The pattern is documented in Feishu's own `99991672` error response:
//! <https://open.feishu.cn/document/faq/trouble-shooting/how-to-fix-the-99991672-error>
//!
//! Lark international (`open.larksuite.com`) accepts the same path layout.
//!
//! Used by:
//! - Channel bots (`handlers/channel_bots.rs`) — surfaces the URL when a
//!   Lark/Feishu bot is created so the user can grant the receive/send
//!   message scopes that NyxID's adapter relies on.
//! - AI services (`handlers/keys.rs`) — surfaces the URL on Lark/Feishu
//!   bot-tenant keys (`api-lark-bot`, `api-feishu-bot`) so the user can
//!   pre-select the scopes their proxied API calls require.

/// Which Lark variant the URL should target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LarkRegion {
    /// Lark international — `open.larksuite.com`.
    International,
    /// Feishu (China mainland) — `open.feishu.cn`.
    China,
}

impl LarkRegion {
    fn open_platform_base(self) -> &'static str {
        match self {
            LarkRegion::International => "https://open.larksuite.com",
            LarkRegion::China => "https://open.feishu.cn",
        }
    }
}

/// Map a channel bot platform identifier to the Lark region. Returns `None`
/// for non-Lark platforms so callers can short-circuit.
pub fn region_for_channel_platform(platform: &str) -> Option<LarkRegion> {
    match platform {
        "lark" => Some(LarkRegion::International),
        "feishu" => Some(LarkRegion::China),
        _ => None,
    }
}

/// Map a catalog (`DownstreamService`) slug to the Lark region. Used by
/// the `/keys` handler to derive a permission URL straight from the
/// catalog entry without a separate `ProviderConfig` lookup.
pub fn region_for_catalog_service_slug(slug: &str) -> Option<LarkRegion> {
    match slug {
        "api-lark" | "api-lark-bot" => Some(LarkRegion::International),
        "api-feishu" | "api-feishu-bot" => Some(LarkRegion::China),
        _ => None,
    }
}

/// Build the Feishu / Lark permission setup URL for the given app.
///
/// `scopes` may be empty, in which case the `q` parameter is omitted and
/// the URL still deep-links the developer to the right page (no
/// pre-selection). Each scope string is URL-encoded individually so a
/// scope like `im:message:send_as_bot` survives the colon → `%3A`
/// transform without breaking the comma separator that Feishu expects.
pub fn build_permission_setup_url(region: LarkRegion, app_id: &str, scopes: &[&str]) -> String {
    let base = region.open_platform_base();
    let encoded_app_id = urlencoding::encode(app_id);
    if scopes.is_empty() {
        return format!("{base}/app/{encoded_app_id}/auth?op_from=openapi");
    }
    let q = scopes
        .iter()
        .map(|s| urlencoding::encode(s).into_owned())
        .collect::<Vec<_>>()
        .join(",");
    format!("{base}/app/{encoded_app_id}/auth?q={q}&op_from=openapi")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_for_channel_platform_maps_known_platforms() {
        assert_eq!(
            region_for_channel_platform("lark"),
            Some(LarkRegion::International)
        );
        assert_eq!(
            region_for_channel_platform("feishu"),
            Some(LarkRegion::China)
        );
        assert_eq!(region_for_channel_platform("telegram"), None);
        assert_eq!(region_for_channel_platform(""), None);
    }

    #[test]
    fn region_for_catalog_service_slug_covers_oauth_and_bot_seeds() {
        assert_eq!(
            region_for_catalog_service_slug("api-lark"),
            Some(LarkRegion::International)
        );
        assert_eq!(
            region_for_catalog_service_slug("api-lark-bot"),
            Some(LarkRegion::International)
        );
        assert_eq!(
            region_for_catalog_service_slug("api-feishu"),
            Some(LarkRegion::China)
        );
        assert_eq!(
            region_for_catalog_service_slug("api-feishu-bot"),
            Some(LarkRegion::China)
        );
        assert_eq!(region_for_catalog_service_slug("api-openai"), None);
    }

    #[test]
    fn build_url_international_with_scopes() {
        let url = build_permission_setup_url(
            LarkRegion::International,
            "cli_a40bc75349bcfff1",
            &["im:message", "im:message:send_as_bot"],
        );
        assert_eq!(
            url,
            "https://open.larksuite.com/app/cli_a40bc75349bcfff1/auth?q=im%3Amessage,im%3Amessage%3Asend_as_bot&op_from=openapi"
        );
    }

    #[test]
    fn build_url_china_with_scopes() {
        let url = build_permission_setup_url(
            LarkRegion::China,
            "cli_a40bc75349bcfff1",
            &["contact:user.id:readonly"],
        );
        assert_eq!(
            url,
            "https://open.feishu.cn/app/cli_a40bc75349bcfff1/auth?q=contact%3Auser.id%3Areadonly&op_from=openapi"
        );
    }

    #[test]
    fn build_url_empty_scopes_omits_q_parameter() {
        let url = build_permission_setup_url(LarkRegion::International, "cli_test", &[]);
        assert_eq!(
            url,
            "https://open.larksuite.com/app/cli_test/auth?op_from=openapi"
        );
    }

    #[test]
    fn build_url_url_encodes_app_id_with_unsafe_chars() {
        // Defensive: app_id should be alphanumeric in practice, but if a
        // caller passes anything containing characters like `/` or `?`
        // they must be encoded so the path stays unambiguous.
        let url = build_permission_setup_url(LarkRegion::International, "weird/id?x", &[]);
        assert!(url.starts_with("https://open.larksuite.com/app/weird%2Fid%3Fx/auth"));
    }

    #[test]
    fn build_url_preserves_scope_order() {
        // The order matters for human readability and lets the UI mirror
        // the catalog's declared order.
        let url =
            build_permission_setup_url(LarkRegion::International, "id", &["c:c", "a:a", "b:b"]);
        assert!(url.contains("q=c%3Ac,a%3Aa,b%3Ab"));
    }

    #[test]
    fn build_url_keeps_duplicate_scopes_verbatim() {
        // Dedup is the caller's job: keeping duplicates verbatim makes
        // the helper a transparent renderer and lets tests / debugging
        // see exactly what the catalog declared. Feishu's permissions
        // page tolerates dupes — the second checkbox flip is a no-op.
        let url = build_permission_setup_url(
            LarkRegion::International,
            "id",
            &["im:message", "im:message", "im:message:send_as_bot"],
        );
        assert!(url.contains("q=im%3Amessage,im%3Amessage,im%3Amessage%3Asend_as_bot"));
    }

    #[test]
    fn build_url_encodes_literal_comma_in_a_scope() {
        // Real Lark scope keys never contain commas, but if the
        // catalog ever stores a malformed entry the literal comma
        // must be percent-encoded so it can't be mistaken for the
        // separator between adjacent scopes in `q=`.
        let url =
            build_permission_setup_url(LarkRegion::International, "id", &["weird,scope", "ok"]);
        assert!(
            url.contains("q=weird%2Cscope,ok"),
            "literal comma inside a scope should be encoded as %2C, got: {url}"
        );
    }
}

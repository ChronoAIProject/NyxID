use axum::{
    body::Body,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};

fn escaped_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

struct ValidatedDestination<'a> {
    raw: &'a str,
    parsed: url::Url,
}

fn admitted_destination(destination: &str) -> Option<ValidatedDestination<'_>> {
    let parsed = if destination.starts_with('/')
        && !destination.starts_with("//")
        && crate::services::oauth_service::normalize_redirect_uri(destination) == destination
        && !destination.contains('\\')
    {
        url::Url::parse("https://relative.invalid")
            .ok()?
            .join(destination)
            .ok()?
    } else {
        crate::services::oauth_service::validate_redirect_uri_scheme(destination).ok()?
    };
    Some(ValidatedDestination {
        raw: destination,
        parsed,
    })
}

pub(crate) fn redirect_page_html(
    destination: &str,
    title: &str,
    message: &str,
    delay_seconds: u8,
) -> Option<String> {
    let destination = admitted_destination(destination)?;
    render_validated_destination(&destination, title, message, delay_seconds)
}

fn render_validated_destination(
    destination: &ValidatedDestination<'_>,
    title: &str,
    message: &str,
    delay_seconds: u8,
) -> Option<String> {
    // Keep the parsed destination through rendering and defend the final sink.
    crate::services::oauth_service::validate_redirect_uri_scheme(destination.parsed.as_str())
        .ok()?;
    let escaped_destination = escaped_attribute(destination.raw);
    let escaped_title = escaped_attribute(title);
    let escaped_message = escaped_attribute(message);
    Some(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"referrer\" content=\"no-referrer\">\
         <meta http-equiv=\"refresh\" content=\"{delay_seconds};url={escaped_destination}\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>NyxID — {escaped_title}</title><style>\
         *{{box-sizing:border-box}}body{{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;background:#0D0D0D;color:#FAFAFA;font-family:-apple-system,BlinkMacSystemFont,\"Segoe UI\",sans-serif;line-height:1.4}}\
         .wrap{{width:100%;max-width:460px;padding:24px;text-align:center}}.brand{{font-weight:700;letter-spacing:.04em;margin-bottom:24px}}\
         .card{{padding:28px;border:1px solid rgba(16,185,129,.3);border-radius:12px;background:rgba(16,185,129,.05)}}\
         h1{{font-size:18px;margin:0 0 8px}}p{{font-size:14px;color:#A3A3A3;margin:0 0 20px}}\
         .bar{{height:3px;background:#262626;border-radius:2px;margin:20px 0;overflow:hidden}}.fill{{height:100%;width:100%;background:#10B981}}\
         a{{color:#A672FB;font-size:14px}}.foot{{font-size:12px;color:#737373;margin-top:18px}}\
         </style></head><body><main class=\"wrap\"><div class=\"brand\">NyxID</div><section class=\"card\">\
         <h1>{escaped_title}</h1><p>{escaped_message}</p><div class=\"bar\"><div class=\"fill\"></div></div>\
         <a href=\"{escaped_destination}\">Continue to the application</a></section>\
         <p class=\"foot\">You can close this tab if it doesn&rsquo;t redirect automatically.</p></main></body></html>"
    ))
}

pub(crate) fn redirect_interstitial(
    destination: &str,
    title: &str,
    message: &str,
    delay_seconds: u8,
) -> Response {
    let Some(html) = redirect_page_html(destination, title, message, delay_seconds) else {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-store")
            .body(Body::from("Invalid redirect destination"))
            .expect("static response is valid");
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .header(header::REFERRER_POLICY, "no-referrer")
        .header(
            header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'none'",
        )
        .body(Body::from(html))
        .expect("static response headers are valid")
}

pub(crate) fn mark_browser_form_response(mut response: Response) -> Response {
    response.headers_mut().insert(
        "x-nyxid-browser-consent-form",
        HeaderValue::from_static("1"),
    );
    response
}

/// Finish a consent form POST before navigating to its validated destination.
/// Chromium applies the submitting page's `form-action 'self'` to HTTP redirect
/// chains too: a same-origin POST followed by a cross-origin 302 is blocked.
/// A 200 document ends the form submission; its meta refresh is a new navigation.
/// Apply this only to the consent POST routes, after their authority checks.
pub(crate) async fn complete_form_post(
    request_headers: HeaderMap,
    mut response: Response,
) -> Response {
    // Fetch Metadata is only content negotiation here; it grants no authority.
    // API clients retain the endpoint's redirect response contract. Axum's
    // map_response runs this request-parts extractor before the handler.
    let browser_form_marker = response
        .headers()
        .contains_key("x-nyxid-browser-consent-form");
    response
        .headers_mut()
        .remove("x-nyxid-browser-consent-form");
    let document_navigation = browser_form_marker
        || (request_headers
            .get("sec-fetch-mode")
            .is_some_and(|value| value == "navigate")
            && request_headers
                .get("sec-fetch-dest")
                .is_some_and(|value| value == "document"));
    // Match the authorization handler's explicit JSON negotiation even if a
    // caller supplies navigation metadata alongside its API Accept header.
    let accepts_json = request_headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("application/json"));
    let accepts_html = request_headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/html"));
    if ((!document_navigation && !browser_form_marker && !accepts_html) || accepts_json)
        || !matches!(response.status(), StatusCode::FOUND | StatusCode::SEE_OTHER)
    {
        return response;
    }
    let Some(destination) = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
    else {
        return response;
    };
    let Some(html) = redirect_page_html(
        destination,
        "Returning to the application",
        "Redirecting to the application…",
        0,
    ) else {
        // A consent form response must never fall back to a redirect that the
        // shared renderer rejected. The handlers validate this before issuing
        // the response; keep the second check fail-closed if a future path
        // accidentally supplies an unsafe Location.
        *response.status_mut() = StatusCode::BAD_REQUEST;
        let headers = response.headers_mut();
        headers.remove(header::LOCATION);
        headers.remove(header::CONTENT_LENGTH);
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/plain; charset=utf-8"),
        );
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        headers.insert(
            header::REFERRER_POLICY,
            HeaderValue::from_static("no-referrer"),
        );
        *response.body_mut() = Body::from("Invalid redirect destination");
        return response;
    };

    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.remove(header::LOCATION);
    headers.remove(header::CONTENT_LENGTH);
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(
            "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'none'",
    ));
    *response.body_mut() = Body::from(html);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::to_bytes, http::Request, middleware, routing::post};
    use tower::ServiceExt;

    fn navigation_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("sec-fetch-mode", HeaderValue::from_static("navigate"));
        headers.insert("sec-fetch-dest", HeaderValue::from_static("document"));
        headers
    }

    fn redirect(destination: &str) -> Response {
        Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, destination)
            .header(header::CONTENT_LENGTH, "0")
            .header(header::SET_COOKIE, "fixture=value; HttpOnly; Secure")
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn completes_allow_deny_error_and_login_redirects_without_a_redirect_chain() {
        for destination in [
            "https://client.example/callback?code=fixture&state=original",
            "https://client.example/callback?error=access_denied&state=original",
            "https://client.example/callback?error=server_error&state=original",
            "/login?return_to=%2Foauth%2Fauthorize",
        ] {
            let response = complete_form_post(navigation_headers(), redirect(destination)).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert!(!response.headers().contains_key(header::LOCATION));
            assert!(!response.headers().contains_key(header::CONTENT_LENGTH));
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
            assert_eq!(
                response.headers()[header::SET_COOKIE],
                "fixture=value; HttpOnly; Secure"
            );
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                "text/html; charset=utf-8"
            );
            let csp = response.headers()[header::CONTENT_SECURITY_POLICY]
                .to_str()
                .unwrap();
            assert!(csp.contains("default-src 'none'"));
            assert!(csp.contains("style-src 'unsafe-inline'"));
            assert!(csp.contains("form-action 'none'"));
            let body = to_bytes(response.into_body(), 4096).await.unwrap();
            let html = std::str::from_utf8(&body).unwrap();
            assert!(html.contains(&format!(
                "content=\"0;url={}\"",
                destination.replace('&', "&amp;")
            )));
            assert!(!html.contains("<script"));
        }
    }

    #[tokio::test]
    async fn escapes_callback_markup_without_executable_script() {
        let response = complete_form_post(
            navigation_headers(),
            redirect(
                "https://client.example/callback?state=\"><script>fixture</script>&code=fixture",
            ),
        )
        .await;
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        let html = std::str::from_utf8(&body).unwrap();
        assert!(html.contains("state=&quot;&gt;&lt;script&gt;"));
        assert!(html.contains("&amp;code=fixture"));
        assert!(!html.contains("<script"));
    }

    #[tokio::test]
    async fn preserves_existing_success_pages_and_non_redirect_errors() {
        for status in [
            StatusCode::OK,
            StatusCode::BAD_REQUEST,
            StatusCode::UNAUTHORIZED,
        ] {
            let response = Response::builder()
                .status(status)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("original response"))
                .unwrap();
            let response = complete_form_post(navigation_headers(), response).await;
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
            assert_eq!(
                to_bytes(response.into_body(), 4096).await.unwrap(),
                "original response"
            );
        }
    }

    #[tokio::test]
    async fn rejects_unsafe_browser_redirect_instead_of_forwarding_location() {
        let response =
            complete_form_post(navigation_headers(), redirect("javascript:fixture")).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(!response.headers().contains_key(header::LOCATION));
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "text/plain; charset=utf-8"
        );

        assert!(redirect_page_html("/login?return_to=%2Foauth", "title", "message", 0).is_some());
        assert!(redirect_page_html("/\\\\evil", "title", "message", 0).is_none());
    }

    #[tokio::test]
    async fn consent_post_layer_leaves_authorization_get_redirects_intact() {
        async fn handler() -> Response {
            redirect("https://client.example/callback?code=fixture&state=original")
        }
        let app = Router::new()
            .route("/oauth/authorize", axum::routing::get(handler))
            .route(
                "/oauth/authorize/decision",
                post(handler).layer(middleware::map_response(complete_form_post)),
            );
        for (method, uri, navigation, expected) in [
            ("GET", "/oauth/authorize", true, StatusCode::FOUND),
            ("POST", "/oauth/authorize/decision", true, StatusCode::OK),
            (
                "POST",
                "/oauth/authorize/decision",
                false,
                StatusCode::FOUND,
            ),
        ] {
            let mut request = Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap();
            if navigation {
                *request.headers_mut() = navigation_headers();
            }
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), expected);
        }
    }

    #[tokio::test]
    async fn preserves_non_navigation_redirects_and_method_preserving_statuses() {
        let destination = "https://client.example/callback?code=fixture";
        for headers in [HeaderMap::new(), {
            let mut headers = navigation_headers();
            headers.insert("sec-fetch-mode", HeaderValue::from_static("cors"));
            headers
        }] {
            let response = complete_form_post(headers, redirect(destination)).await;
            assert_eq!(response.status(), StatusCode::FOUND);
            assert_eq!(response.headers()[header::LOCATION], destination);
        }
        for (status, expected) in [
            (StatusCode::SEE_OTHER, StatusCode::OK),
            (
                StatusCode::TEMPORARY_REDIRECT,
                StatusCode::TEMPORARY_REDIRECT,
            ),
            (
                StatusCode::PERMANENT_REDIRECT,
                StatusCode::PERMANENT_REDIRECT,
            ),
            (StatusCode::NOT_MODIFIED, StatusCode::NOT_MODIFIED),
        ] {
            let mut response = redirect(destination);
            *response.status_mut() = status;
            let response = complete_form_post(navigation_headers(), response).await;
            assert_eq!(response.status(), expected);
        }
    }

    #[tokio::test]
    async fn preserves_explicit_json_contract_even_with_navigation_metadata() {
        let mut headers = navigation_headers();
        headers.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        let destination = "https://client.example/callback?code=fixture";
        let response = complete_form_post(headers.clone(), redirect(destination)).await;
        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(response.headers()[header::LOCATION], destination);

        let response = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{\"redirect_url\":\"fixture\"}"))
            .unwrap();
        let response = complete_form_post(headers, response).await;
        assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
        assert_eq!(
            to_bytes(response.into_body(), 4096).await.unwrap(),
            "{\"redirect_url\":\"fixture\"}"
        );
    }

    #[tokio::test]
    async fn accepts_legacy_html_form_without_fetch_metadata() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static("text/html"));
        let response = complete_form_post(
            headers,
            redirect("https://client.example/callback?state=legacy"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }
}

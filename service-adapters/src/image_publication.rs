//! Provider wire contracts. File access, human consent and replay barriers belong
//! to the host. A temporary URL is accepted only from authenticated initialization.
use http::HeaderMap;
use serde_json::{Value, json};

pub type Result<T> = std::result::Result<T, &'static str>;

pub struct ApiRequest {
    pub method: &'static str,
    pub path: String,
    pub query: Option<String>,
    pub body: Option<Value>,
}

#[derive(Clone, Copy)]
pub enum UploadCredential {
    None,
    SelectedBearer,
}

pub struct Upload {
    pub url: String,
    pub image: String,
    pub expires_at_ms: i64,
    pub credential: UploadCredential,
    pub headers: Vec<(String, String)>,
}
impl std::fmt::Debug for Upload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Upload([REDACTED])")
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Readiness {
    Pending,
    Ready,
    Failed,
}

/// An adapter cannot acquire credentials, authorize files or perform network IO.
/// Each request still passes the host's live authority and destination fences.
pub trait ImagePublisher: Send + Sync {
    fn api_origin(&self) -> &'static str;
    fn company(&self, value: &str) -> bool;
    fn company_url(&self, value: &str) -> Option<String>;
    fn headers(&self) -> Vec<(&'static str, &'static str)>;
    fn initialize(&self, company: &str) -> ApiRequest;
    fn upload(&self, body: &Value) -> Result<Upload>;
    fn permits_upload(&self, url: &str) -> bool;
    fn image_status(&self, image: &str) -> ApiRequest;
    fn readiness(&self, body: &Value, company: &str, image: &str) -> Result<Readiness>;
    fn post(&self, company: &str, caption: &str, image: &str) -> ApiRequest;
    fn receipt(&self, headers: &HeaderMap) -> Option<String>;
    fn readback(&self, post: &str) -> ApiRequest;
    fn feed_url(&self, post: &str) -> Option<String>;
    fn wire_caption(&self, caption: &str) -> String;
    fn readback_caption(
        &self,
        body: &Value,
        post: &str,
        company: &str,
        image: &str,
    ) -> Result<String>;
    fn verify(&self, body: &Value, post: &str, company: &str, caption: &str, image: &str) -> bool {
        self.readback_caption(body, post, company, image)
            .is_ok_and(|wire| wire == self.wire_caption(caption))
    }
}

pub struct LinkedIn;
pub const LINKEDIN_VERSION: &str = "202611";

pub fn organization(value: &str) -> bool {
    numeric_urn(value, "organization")
}
pub fn post_urn(value: &str) -> bool {
    numeric_urn(value, "share") || numeric_urn(value, "ugcPost")
}
fn numeric_urn(value: &str, kind: &str) -> bool {
    value
        .strip_prefix(&format!("urn:li:{kind}:"))
        .is_some_and(|id| {
            !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_digit())
        })
}
pub fn image_urn(value: &str) -> bool {
    value.strip_prefix("urn:li:image:").is_some_and(|id| {
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    })
}
pub fn feed_url(post: &str) -> Option<String> {
    post_urn(post).then(|| format!("https://www.linkedin.com/feed/update/{post}/"))
}

/// Exact HTTPS origins and upload path families, without suffix-host matching.
pub fn upload_recipient(raw: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(raw) else {
        return false;
    };
    url.scheme() == "https"
        && url.port().is_none_or(|p| p == 443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.fragment().is_none()
        && matches!(
            url.host_str(),
            Some("www.linkedin.com" | "api.linkedin.com")
        )
        && url.path().starts_with("/dms-uploads/")
        && !url.path().contains('%')
        && !url.path().contains('\\')
}

fn request(
    method: &'static str,
    path: &str,
    query: Option<&str>,
    body: Option<Value>,
) -> ApiRequest {
    ApiRequest {
        method,
        path: path.into(),
        query: query.map(str::to_owned),
        body,
    }
}
/// Escape little-text syntax so the visible text remains the owner's literal caption.
pub fn little_text(caption: &str) -> String {
    let mut out = String::with_capacity(caption.len());
    for ch in caption.chars() {
        if "\\|{}@[]()<>#*_~".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}
impl ImagePublisher for LinkedIn {
    fn api_origin(&self) -> &'static str {
        "https://api.linkedin.com"
    }
    fn company(&self, value: &str) -> bool {
        organization(value)
    }
    fn company_url(&self, value: &str) -> Option<String> {
        organization(value).then(|| {
            format!(
                "https://www.linkedin.com/company/{}/",
                value
                    .strip_prefix("urn:li:organization:")
                    .unwrap_or_default()
            )
        })
    }
    fn headers(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("LinkedIn-Version", LINKEDIN_VERSION),
            ("X-Restli-Protocol-Version", "2.0.0"),
        ]
    }
    fn initialize(&self, company: &str) -> ApiRequest {
        request(
            "POST",
            "/rest/images",
            Some("action=initializeUpload"),
            Some(json!({"initializeUploadRequest":{"owner":company}})),
        )
    }
    fn permits_upload(&self, url: &str) -> bool {
        upload_recipient(url)
    }
    fn image_status(&self, image: &str) -> ApiRequest {
        request(
            "GET",
            &format!("/rest/images/{}", urlencoding::encode(image)),
            None,
            None,
        )
    }
    fn upload(&self, body: &Value) -> Result<Upload> {
        let value = &body["value"];
        let image = value["image"]
            .as_str()
            .filter(|s| image_urn(s))
            .ok_or("invalid_image")?;
        let url = value["uploadUrl"]
            .as_str()
            .filter(|s| upload_recipient(s))
            .ok_or("invalid_upload_recipient")?;
        let expiry = value["uploadUrlExpiresAt"]
            .as_i64()
            .ok_or("missing_upload_expiry")?;
        Ok(Upload {
            url: url.into(),
            image: image.into(),
            expires_at_ms: expiry,
            credential: UploadCredential::SelectedBearer,
            headers: Vec::new(),
        })
    }
    fn readiness(&self, body: &Value, company: &str, image: &str) -> Result<Readiness> {
        if body["owner"] != company || body["id"] != image {
            return Err("image_owner_mismatch");
        }
        match body["status"].as_str() {
            Some("AVAILABLE") => Ok(Readiness::Ready),
            Some("WAITING_UPLOAD" | "PROCESSING") => Ok(Readiness::Pending),
            Some("PROCESSING_FAILED" | "UPLOAD_FAILED") => Ok(Readiness::Failed),
            _ => Err("invalid_image_status"),
        }
    }
    fn post(&self, company: &str, caption: &str, image: &str) -> ApiRequest {
        request(
            "POST",
            "/rest/posts",
            None,
            Some(
                json!({"author":company,"commentary":little_text(caption),"visibility":"PUBLIC",
            "distribution":{"feedDistribution":"MAIN_FEED","targetEntities":[],"thirdPartyDistributionChannels":[]},
            "content":{"media":{"id":image}},"lifecycleState":"PUBLISHED","isReshareDisabledByAuthor":false}),
            ),
        )
    }
    fn receipt(&self, headers: &HeaderMap) -> Option<String> {
        let mut values = headers.get_all("x-restli-id").iter();
        let value = values.next()?.to_str().ok()?;
        if values.next().is_some() || !post_urn(value) {
            return None;
        }
        Some(value.into())
    }
    fn readback(&self, post: &str) -> ApiRequest {
        request(
            "GET",
            &format!("/rest/posts/{}", urlencoding::encode(post)),
            None,
            None,
        )
    }
    fn feed_url(&self, post: &str) -> Option<String> {
        feed_url(post)
    }
    fn wire_caption(&self, caption: &str) -> String {
        little_text(caption)
    }
    fn readback_caption(
        &self,
        body: &Value,
        post: &str,
        company: &str,
        image: &str,
    ) -> Result<String> {
        if body.get("id").is_none_or(|id| id == post)
            && body["author"] == company
            && body["content"]["media"]["id"] == image
            && body["lifecycleState"] == "PUBLISHED"
        {
            return body["commentary"]
                .as_str()
                .map(str::to_owned)
                .ok_or("readback_caption");
        }
        Err("readback_mismatch")
    }
}

/// A second wire format, available only to the host's mock tests.
#[cfg(any(test, feature = "test-support"))]
pub struct FixturePublisher;
#[cfg(any(test, feature = "test-support"))]
impl ImagePublisher for FixturePublisher {
    fn api_origin(&self) -> &'static str {
        "https://fixture.publisher.test"
    }
    fn company(&self, value: &str) -> bool {
        value.starts_with("page:")
    }
    fn company_url(&self, value: &str) -> Option<String> {
        Some(format!("https://fixture.publisher.test/{value}"))
    }
    fn headers(&self) -> Vec<(&'static str, &'static str)> {
        vec![("fixture-version", "1")]
    }
    fn initialize(&self, company: &str) -> ApiRequest {
        request("POST", "/files", None, Some(json!({"account":company})))
    }
    fn upload(&self, body: &Value) -> Result<Upload> {
        Ok(Upload {
            url: body["put"]
                .as_str()
                .filter(|s| self.permits_upload(s))
                .ok_or("fixture_recipient")?
                .into(),
            image: body["file"].as_str().ok_or("fixture_file")?.into(),
            expires_at_ms: body["until"].as_i64().ok_or("fixture_expiry")?,
            credential: UploadCredential::None,
            headers: vec![("fixture-upload".into(), "original".into())],
        })
    }
    fn permits_upload(&self, url: &str) -> bool {
        reqwest::Url::parse(url).is_ok_and(|url| {
            url.origin().ascii_serialization() == "https://fixture.upload.test"
                && url.path() == "/bytes"
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none()
        })
    }
    fn image_status(&self, image: &str) -> ApiRequest {
        request("GET", &format!("/files/{image}"), None, None)
    }
    fn readiness(&self, body: &Value, company: &str, image: &str) -> Result<Readiness> {
        if body["account"] != company || body["file"] != image {
            return Err("fixture_owner");
        }
        Ok(if body["ready"] == true {
            Readiness::Ready
        } else {
            Readiness::Pending
        })
    }
    fn post(&self, company: &str, caption: &str, image: &str) -> ApiRequest {
        request(
            "POST",
            "/entries",
            None,
            Some(json!({"account":company,"text":caption,"file":image})),
        )
    }
    fn receipt(&self, headers: &HeaderMap) -> Option<String> {
        headers
            .get("fixture-id")?
            .to_str()
            .ok()
            .filter(|s| s.starts_with("entry-"))
            .map(str::to_owned)
    }
    fn readback(&self, post: &str) -> ApiRequest {
        request("GET", &format!("/entries/{post}"), None, None)
    }
    fn feed_url(&self, post: &str) -> Option<String> {
        Some(format!("https://fixture.publisher.test/entries/{post}"))
    }
    fn wire_caption(&self, caption: &str) -> String {
        caption.into()
    }
    fn readback_caption(
        &self,
        body: &Value,
        post: &str,
        company: &str,
        image: &str,
    ) -> Result<String> {
        if body["id"] == post && body["account"] == company && body["file"] == image {
            return body["text"]
                .as_str()
                .map(str::to_owned)
                .ok_or("fixture_caption");
        }
        Err("fixture_mismatch")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linkedin_recipient_and_header_receipt_are_closed() {
        for url in [
            "https://www.linkedin.com/dms-uploads/a?x=1%2F2",
            "https://api.linkedin.com/dms-uploads/a",
        ] {
            assert!(upload_recipient(url));
        }
        for url in [
            "http://www.linkedin.com/dms-uploads/a",
            "https://www.linkedin.com.evil/dms-uploads/a",
            "https://www.linkedin.com:444/dms-uploads/a",
            "https://user@www.linkedin.com/dms-uploads/a",
            "https://www.linkedin.com/other",
            "https://www.linkedin.com/dms-uploads/a#secret",
        ] {
            assert!(!upload_recipient(url));
        }
        let mut headers = HeaderMap::new();
        headers.insert("x-restli-id", "urn:li:share:1234".parse().unwrap());
        assert_eq!(
            LinkedIn.receipt(&headers).as_deref(),
            Some("urn:li:share:1234")
        );
        headers.insert("x-restli-id", "https://evil.test".parse().unwrap());
        assert_eq!(LinkedIn.receipt(&headers), None);
    }
    #[test]
    fn linkedin_literal_caption_and_readback_identity() {
        let caption = "Hi @Rona #news | [a](b) \\ *bold*";
        let mut body = LinkedIn
            .post("urn:li:organization:42", caption, "urn:li:image:abc")
            .body
            .unwrap();
        assert_ne!(body["commentary"], caption);
        body["id"] = json!("urn:li:share:1");
        assert!(LinkedIn.verify(
            &body,
            "urn:li:share:1",
            "urn:li:organization:42",
            caption,
            "urn:li:image:abc"
        ));
        assert!(!LinkedIn.verify(
            &body,
            "urn:li:share:2",
            "urn:li:organization:42",
            caption,
            "urn:li:image:abc"
        ));
    }
}

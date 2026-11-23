//! Owned credential-bearing transport fields. Serde uses their original string representation.
use serde::{Deserialize, Serialize};
use zeroize::{ZeroizeOnDrop, Zeroizing};

#[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ZeroizeOnDrop)]
#[serde(transparent)]
pub struct SecretText(String);

impl SecretText {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn concat(parts: &[&str]) -> Self {
        Self(parts.concat())
    }

    pub fn with_query_param(url: &str, name: &str, value: &str) -> Self {
        let separator = if url.contains('?') { "&" } else { "?" };
        let name = urlencoding::encode(name);
        match urlencoding::encode(value) {
            std::borrow::Cow::Borrowed(value) => Self::concat(&[url, separator, &name, "=", value]),
            std::borrow::Cow::Owned(value) => {
                let value = zeroize::Zeroizing::new(value);
                Self::concat(&[url, separator, &name, "=", &value])
            }
        }
    }
}

impl From<String> for SecretText {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for SecretText {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl std::ops::Deref for SecretText {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for SecretText {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretText([REDACTED])")
    }
}

/// Render an application-owned reply in a single allocation, including repeated placeholders.
pub fn render_credential_frame(template: &str, credential: &str) -> Zeroizing<Vec<u8>> {
    const PLACEHOLDER: &str = "${credential}";
    let count = template.matches(PLACEHOLDER).count();
    let capacity = template.len() - count * PLACEHOLDER.len() + count * credential.len();
    let mut bytes = Zeroizing::new(Vec::with_capacity(capacity));
    let mut parts = template.split(PLACEHOLDER);
    bytes.extend_from_slice(parts.next().unwrap_or_default().as_bytes());
    for part in parts {
        bytes.extend_from_slice(credential.as_bytes());
        bytes.extend_from_slice(part.as_bytes());
    }
    bytes
}

//! URL references carried in downstream HTTP response headers.

pub const HEADERS: [&str; 3] = ["location", "operation-location", "content-location"];

/// Compare octets so arbitrary signature bytes may accompany a URL credential.
pub fn carries_credential(reference: &str, credential: &str) -> bool {
    fn contains_decoded(reference: &[u8], credential: &[u8]) -> bool {
        let mut bytes = std::borrow::Cow::Borrowed(reference);
        loop {
            if bytes
                .windows(credential.len())
                .any(|part| part == credential)
            {
                return true;
            }
            match urlencoding::decode_binary(&bytes) {
                std::borrow::Cow::Owned(next) if next.len() < bytes.len() => bytes = next.into(),
                _ => return false,
            }
        }
    }

    !credential.is_empty()
        && (contains_decoded(reference.as_bytes(), credential.as_bytes())
            || (credential.contains(' ')
                && reference.contains('+')
                && contains_decoded(
                    reference.replace('+', " ").as_bytes(),
                    credential.as_bytes(),
                )))
}

use super::TlsError;

pub(super) fn nonempty_env(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name).filter(|value| !value.is_empty())
}

// Only CA paths are persisted; proxy environment values may contain credentials.
pub(crate) fn ca_environment_from_env() -> Result<Vec<(&'static str, String)>, TlsError> {
    absolute_ca_environment(|name| std::env::var_os(name))
}

pub(crate) fn absolute_ca_environment(
    get: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Result<Vec<(&'static str, String)>, TlsError> {
    let mut result = Vec::new();
    for name in ["NYXID_CA_CERT", "SSL_CERT_FILE", "SSL_CERT_DIR"] {
        if let Some(value) = get(name).filter(|value| !value.is_empty()) {
            let absolute = if name == "SSL_CERT_DIR" {
                let paths = std::env::split_paths(&value)
                    .filter(|p| !p.as_os_str().is_empty())
                    .map(std::path::absolute)
                    .collect::<std::io::Result<Vec<_>>>()
                    .map_err(|error| TlsError(format!("{name}: cannot resolve path: {error}")))?;
                if paths.is_empty() {
                    return Err(TlsError(
                        "SSL_CERT_DIR has no certificate directories".into(),
                    ));
                }
                std::env::join_paths(paths)
                    .map_err(|_| TlsError("SSL_CERT_DIR contains an invalid path".into()))?
            } else {
                std::path::absolute(value)
                    .map_err(|error| TlsError(format!("{name}: cannot resolve path: {error}")))?
                    .into_os_string()
            };
            let value = absolute.into_string().map_err(|_| {
                TlsError(format!(
                    "{name} path must be valid Unicode for the service definition"
                ))
            })?;
            if value
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
            {
                return Err(TlsError(format!(
                    "{name} path contains unsupported control characters"
                )));
            }
            result.push((name, value));
        }
    }
    Ok(result)
}

pub(crate) fn plist_ca_environment(values: &[(&str, String)]) -> String {
    values
        .iter()
        .map(|(name, value)| {
            format!(
                "        <key>{}</key>\n        <string>{}</string>\n",
                xml_escape(name),
                xml_escape(value).replace('\r', "&#13;")
            )
        })
        .collect()
}

pub(crate) fn systemd_ca_environment(values: &[(&str, String)]) -> String {
    values
        .iter()
        .map(|(name, value)| {
            let value = value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('%', "%%")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
                .replace('\t', "\\t");
            format!("Environment=\"{name}={value}\"\n")
        })
        .collect()
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_ca_paths_are_omitted_from_both_service_formats() {
        let empty = absolute_ca_environment(|_| Some("".into())).unwrap();
        assert!(empty.is_empty());
        assert!(plist_ca_environment(&empty).is_empty());
        assert!(systemd_ca_environment(&empty).is_empty());
    }
}

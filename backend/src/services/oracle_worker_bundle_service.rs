use std::sync::LazyLock;

use sha2::{Digest, Sha256};

const WORKER_SOURCE: &str = include_str!("../../../integrations/oracle/cdp-worker/worker.mjs");

pub struct WorkerBundle {
    pub version: &'static str,
    pub sha256: &'static str,
    pub source: &'static str,
}

static BUNDLE_SHA256: LazyLock<String> =
    LazyLock::new(|| hex::encode(Sha256::digest(WORKER_SOURCE.as_bytes())));
static BUNDLE_VERSION: LazyLock<String> =
    LazyLock::new(|| format!("{}+{}", env!("CARGO_PKG_VERSION"), &BUNDLE_SHA256[..12]));

pub fn current_bundle() -> WorkerBundle {
    WorkerBundle {
        version: BUNDLE_VERSION.as_str(),
        sha256: BUNDLE_SHA256.as_str(),
        source: WORKER_SOURCE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_bundle_checksum_matches_source() {
        let bundle = current_bundle();
        assert_eq!(
            bundle.sha256,
            hex::encode(Sha256::digest(bundle.source.as_bytes()))
        );
        assert!(bundle.version.ends_with(&bundle.sha256[..12]));
        assert!(bundle.source.contains("connectOverCDP"));
    }

    #[test]
    fn embedded_bundle_version_is_valid_worker_metadata() {
        let bundle = current_bundle();
        assert!(super::super::oracle_worker_service::valid_script_version(
            bundle.version
        ));
    }
}

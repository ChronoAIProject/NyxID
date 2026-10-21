// A build-time adapter only. These are the unchanged W6 consumer modules;
// there is no second certificate, Rekor, TUF, issuer or identity verifier.
#[allow(dead_code)]
#[path = "../cli/src/tls.rs"]
mod tls;
#[cfg(unix)]
#[path = "../cli/src/commands/update_attestation/trusted_directory.rs"]
mod trusted_directory;

#[allow(dead_code)]
mod update_attestation {
    include!("../cli/src/commands/update_attestation.rs");

    pub async fn verify_local(path: &std::path::Path) -> anyhow::Result<usize> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Request {
            bundle: std::path::PathBuf,
            identity: String,
            digests: Vec<String>,
        }
        fn read(path: &std::path::Path, maximum: u64) -> anyhow::Result<Vec<u8>> {
            use std::io::Read;
            let metadata = std::fs::symlink_metadata(path)?;
            anyhow::ensure!(
                metadata.is_file() && metadata.len() <= maximum,
                "bounded regular verifier input required"
            );
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take(maximum + 1)
                .read_to_end(&mut bytes)?;
            anyhow::ensure!(
                bytes.len() as u64 <= maximum,
                "verifier input exceeds bound"
            );
            Ok(bytes)
        }
        let request: Request = serde_json::from_slice(&read(path, 256 * 1024)?)?;
        anyhow::ensure!(
            !request.digests.is_empty()
                && request.digests.len() <= 1024
                && request.identity.len() <= 2048
                && request.digests.iter().all(|d| d.len() == 64
                    && d.bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))),
            "invalid verifier digest/identity request"
        );
        let attestation = Attestation {
            bundle: Some(serde_json::from_slice(&read(
                &request.bundle,
                MAX_BUNDLE_BYTES as u64,
            )?)?),
            bundle_url: None,
        };
        let root = trust::load().await?;
        for digest in &request.digests {
            verify_single_attestation(&attestation, &root, digest, &request.identity)?;
        }
        Ok(request.digests.len())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_| anyhow::anyhow!("unable to initialize TLS crypto provider"))?;
    let arguments = std::env::args_os().collect::<Vec<_>>();
    anyhow::ensure!(
        arguments.len() == 2,
        "usage: release-verifier VERIFICATION-REQUEST.json"
    );
    let count = update_attestation::verify_local(std::path::Path::new(&arguments[1])).await?;
    println!("Unchanged W6 verifier authenticated {count} publication subject digests");
    Ok(())
}

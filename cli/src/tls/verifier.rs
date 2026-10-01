use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::client::{
    WebPkiServerVerifier,
    danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
};
use rustls::{DigitallySignedStruct, Error, RootCertStore, SignatureScheme};

type PlatformLoader = Box<dyn FnOnce() -> rustls_native_certs::CertificateResult + Send>;

pub(super) struct LazyPlatform {
    eager_roots: RootCertStore,
    loader: Mutex<Option<PlatformLoader>>,
    loaded: OnceLock<NativeTrust>,
}

impl std::fmt::Debug for LazyPlatform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LazyPlatform")
            .field("loaded", &self.loaded.get().is_some())
            .finish_non_exhaustive()
    }
}

pub(super) struct NativeTrust {
    verifier: Arc<WebPkiServerVerifier>,
    pub certificates: usize,
    pub load_ms: f64,
}

impl LazyPlatform {
    pub fn new(
        roots: RootCertStore,
        loader: impl FnOnce() -> rustls_native_certs::CertificateResult + Send + 'static,
    ) -> Self {
        Self {
            eager_roots: roots,
            loader: Mutex::new(Some(Box::new(loader))),
            loaded: OnceLock::new(),
        }
    }

    pub fn get(&self) -> &NativeTrust {
        self.loaded.get_or_init(|| {
            let start = Instant::now();
            let loader = self
                .loader
                .lock()
                .expect("platform loader lock")
                .take()
                .expect("platform loaded once");
            let native = loader();
            let mut roots = self.eager_roots.clone();
            let (certificates, _) = roots.add_parsable_certificates(native.certs);
            if certificates == 0 || !native.errors.is_empty() {
                tracing::debug!(
                    certificates,
                    "Platform CA store empty or partially unavailable; bundled roots remain enabled"
                );
            }
            NativeTrust {
                verifier: webpki(roots),
                certificates,
                load_ms: start.elapsed().as_secs_f64() * 1000.0,
            }
        })
    }
}

fn webpki(roots: RootCertStore) -> Arc<WebPkiServerVerifier> {
    WebPkiServerVerifier::builder_with_provider(
        Arc::new(roots),
        Arc::new(rustls::crypto::aws_lc_rs::default_provider()),
    )
    .build()
    .expect("bundled roots are always present")
}

pub(super) fn build(
    roots: RootCertStore,
    platform: Option<Arc<LazyPlatform>>,
) -> Arc<dyn ServerCertVerifier> {
    let eager = webpki(roots);
    match platform {
        Some(platform) => Arc::new(FallbackVerifier { eager, platform }),
        None => eager,
    }
}

#[derive(Debug)]
struct FallbackVerifier {
    eager: Arc<WebPkiServerVerifier>,
    platform: Arc<LazyPlatform>,
}

impl ServerCertVerifier for FallbackVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        match self.eager.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Err(Error::InvalidCertificate(_)) => self.platform.get().verifier.verify_server_cert(
                end_entity,
                intermediates,
                server_name,
                ocsp_response,
                now,
            ),
            result => result,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.eager.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        self.eager.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.eager.supported_verify_schemes()
    }
}

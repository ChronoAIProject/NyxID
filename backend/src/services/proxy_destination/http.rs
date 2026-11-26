//! The DNS iterator consumed by the connector contains exactly the checked answers.
//! A live connection remains bound to its peer. Each new connection resolves again.
use std::{
    collections::VecDeque,
    sync::{Arc, LazyLock, Mutex, OnceLock},
    time::Duration,
};

use reqwest::{
    Client, ClientBuilder,
    dns::{Addrs, Name, Resolve, Resolving},
};
use url::Url;

use super::{Access, Failure, POLICY, refused};
use crate::errors::{AppError, AppResult};
use nyxid_service_adapters::{ifttt, ifttt_mcp};

const MAX_PRIVATE_POOLS: usize = 16;

pub(crate) fn connection_failure(error: &reqwest::Error) -> Option<Failure> {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(cause) = source {
        if let Some(failure) = cause.downcast_ref::<Failure>() {
            return Some(*failure);
        }
        source = cause.source();
    }
    None
}

pub(crate) fn denied_address(error: &reqwest::Error) -> bool {
    connection_failure(error) == Some(Failure::AddressNotPermitted)
}

struct SystemDns;
impl Resolve for SystemDns {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let answers = tokio::net::lookup_host((name.as_str().to_owned(), 0)).await?;
            Ok(Box::new(answers) as Addrs)
        })
    }
}

struct CheckedDns {
    lookup: Arc<dyn Resolve>,
    access: Access,
    private_host: Option<String>,
    timeout: Duration,
}

impl Resolve for CheckedDns {
    fn resolve(&self, name: Name) -> Resolving {
        let permitted_name = !super::is_cloud_metadata_host(name.as_str())
            && self
                .private_host
                .as_deref()
                .is_none_or(|host| host == name.as_str());
        let lookup = self.lookup.clone();
        let access = self.access;
        let timeout = self.timeout;
        Box::pin(async move {
            if !permitted_name {
                return Err(Failure::AddressNotPermitted.into());
            }
            let addresses = super::resolve_answers(lookup.resolve(name), timeout).await?;
            if addresses.is_empty() {
                return Err(Failure::ResolutionUnavailable.into());
            }
            if !access.permits_all(&addresses) {
                return Err(Failure::AddressNotPermitted.into());
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

struct Network {
    dns: Arc<dyn Resolve>,
    #[cfg(test)]
    builder: Option<Arc<dyn Fn() -> ClientBuilder + Send + Sync>>,
}

type Cached<T> = OnceLock<Result<T, &'static str>>;

pub(crate) struct Clients {
    lookup: Arc<dyn Resolve>,
    access: Access,
    private_host: Option<String>,
    #[cfg(test)]
    builder: Option<Arc<dyn Fn() -> ClientBuilder + Send + Sync>>,
    ordinary: Cached<Client>,
    media: Cached<Client>,
    verification: Cached<Client>,
    cma: Cached<Client>,
    ifttt: Cached<ifttt::Client>,
    ifttt_mcp: Cached<ifttt_mcp::Client>,
}

impl Clients {
    fn new(network: &Network, access: Access, private_host: Option<String>) -> Self {
        Self {
            lookup: network.dns.clone(),
            access,
            private_host,
            #[cfg(test)]
            builder: network.builder.clone(),
            ordinary: OnceLock::new(),
            media: OnceLock::new(),
            verification: OnceLock::new(),
            cma: OnceLock::new(),
            ifttt: OnceLock::new(),
            ifttt_mcp: OnceLock::new(),
        }
    }

    fn get<'a, T>(
        &self,
        slot: &'a Cached<T>,
        connect_timeout: Duration,
        build: impl FnOnce(ClientBuilder) -> Result<T, reqwest::Error>,
    ) -> AppResult<&'a T> {
        slot.get_or_init(|| {
            let builder = Client::builder().no_proxy();
            #[cfg(test)]
            let builder = self.builder.as_ref().map_or(builder, |custom| custom());
            // Let the resolver return typed evidence before the whole connect budget expires.
            let resolver = CheckedDns {
                lookup: self.lookup.clone(),
                access: self.access,
                private_host: self.private_host.clone(),
                timeout: connect_timeout - Duration::from_millis(250),
            };
            let builder = builder
                .dns_resolver(Arc::new(resolver))
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(connect_timeout);
            build(builder).map_err(|_| "Direct proxy transport unavailable")
        })
        .as_ref()
        .map_err(|e| AppError::Internal((*e).into()))
    }

    pub(crate) fn ordinary(&self) -> AppResult<Client> {
        self.get(
            &self.ordinary,
            Duration::from_secs(10),
            ClientBuilder::build,
        )
        .cloned()
    }

    pub(crate) fn media(&self) -> AppResult<Client> {
        self.get(&self.media, Duration::from_secs(10), |builder| {
            builder.timeout(Duration::from_secs(30)).build()
        })
        .cloned()
    }

    pub(crate) fn verification(&self) -> AppResult<Client> {
        self.get(
            &self.verification,
            Duration::from_secs(3),
            crate::services::credential_verification::build_http_client,
        )
        .cloned()
    }

    pub(crate) fn cma(&self) -> AppResult<Client> {
        self.get(&self.cma, Duration::from_secs(5), ClientBuilder::build)
            .cloned()
    }

    pub(crate) fn ifttt(&self) -> AppResult<&ifttt::Client> {
        self.get(&self.ifttt, Duration::from_secs(10), ifttt::Client::new)
    }

    pub(crate) fn ifttt_mcp(&self) -> AppResult<&ifttt_mcp::Client> {
        self.get(
            &self.ifttt_mcp,
            Duration::from_secs(10),
            ifttt_mcp::Client::new,
        )
    }
}

struct Pools {
    network: Network,
    public: Arc<Clients>,
    strict: Arc<Clients>,
    private: Mutex<VecDeque<(String, Arc<Clients>)>>,
}

impl Pools {
    fn new(network: Network, loopback: bool) -> Self {
        let strict = Arc::new(Clients::new(&network, Access::default(), None));
        let public = if loopback {
            Arc::new(Clients::new(
                &network,
                Access {
                    private: false,
                    loopback,
                },
                None,
            ))
        } else {
            strict.clone()
        };
        Self {
            network,
            public,
            strict,
            private: Mutex::default(),
        }
    }

    fn select(&self, base: &str, url: &Url, access: Access) -> Result<Arc<Clients>, Failure> {
        if !access.private {
            return Ok(if access.loopback {
                &self.public
            } else {
                &self.strict
            }
            .clone());
        }
        // Access::for_request checked the effective base and request origin, including port.
        let mut cache = self
            .private
            .lock()
            .map_err(|_| Failure::TransportUnavailable)?;
        let entry = if let Some(index) = cache.iter().position(|(key, _)| key == base) {
            cache.remove(index).expect("index from the locked cache")
        } else {
            if cache.len() == MAX_PRIVATE_POOLS {
                cache.pop_front();
            }
            (
                base.to_owned(),
                Arc::new(Clients::new(
                    &self.network,
                    access,
                    url.host_str().map(str::to_owned),
                )),
            )
        };
        let client = entry.1.clone();
        cache.push_back(entry);
        Ok(client)
    }
}

static POOLS: LazyLock<Result<Pools, &'static str>> = LazyLock::new(|| {
    Ok(Pools::new(
        Network {
            dns: Arc::new(SystemDns),
            #[cfg(test)]
            builder: None,
        },
        POLICY.as_ref().map_err(|e| *e)?.loopback,
    ))
});

pub(super) fn initialize() -> Result<(), &'static str> {
    POOLS
        .as_ref()
        .map_err(|e| *e)?
        .public
        .ordinary()
        .map(|_| ())
        .map_err(|_| "Direct proxy transport unavailable")
}

/// A caller-supplied delivery has no catalog or development authorization.
pub(crate) fn public_delivery(raw: &str) -> AppResult<(Url, Client)> {
    let url = Url::parse(raw).map_err(|_| refused())?;
    let strict = true;
    #[cfg(test)]
    let strict = strict && crate::services::resource_adapter::test_media_origin(raw).is_none();
    #[cfg(test)]
    let strict = strict
        && !crate::services::channel_media_service::TEST_PINNED_TRANSPORT
            .try_with(|fixture| fixture.target == raw)
            .unwrap_or(false);
    let client = for_url(None, "", &url, strict)?.ordinary()?;
    Ok((url, client))
}

pub(crate) fn signed_webhook(raw: &str) -> AppResult<(Url, Client)> {
    let (url, client) = public_delivery(raw)?;
    if url.scheme() != "https" {
        #[cfg(test)]
        if crate::services::resource_adapter::test_media_origin(raw).is_some()
            || TEST_POOLS.try_with(|_| ()).is_ok()
            || crate::services::channel_media_service::TEST_PINNED_TRANSPORT
                .try_with(|fixture| fixture.target == raw)
                .unwrap_or(false)
        {
            return Ok((url, client));
        }
        return Err(refused());
    }
    Ok((url, client))
}

pub(crate) fn for_url(
    approved: Option<&str>,
    base: &str,
    url: &Url,
    strict: bool,
) -> Result<Arc<Clients>, Failure> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Failure::UrlShape);
    }
    let approval = if strict { None } else { approved };
    let access = Access::for_request(approval, base, url, strict)?;
    access.check_url(url)?;
    #[cfg(test)]
    if let Ok(result) = TEST_POOLS.try_with(|pools| pools.select(base, url, access)) {
        return result;
    }
    #[cfg(test)]
    if let Some(clients) = fixture_clients(url, access) {
        return clients;
    }
    POOLS
        .as_ref()
        .map_err(|_| Failure::UrlShape)?
        .select(base, url, access)
}

#[cfg(test)]
tokio::task_local! { static TEST_POOLS: Pools; }

#[cfg(test)]
pub(super) fn test_lookup(name: &str) -> Option<Resolving> {
    TEST_POOLS
        .try_with(|p| p.network.dns.resolve(name.parse().unwrap()))
        .ok()
}

#[cfg(test)]
pub(crate) async fn with_test_pools<F: std::future::Future>(
    dns: Arc<dyn Resolve>,
    builder: Arc<dyn Fn() -> ClientBuilder + Send + Sync>,
    work: F,
) -> F::Output {
    TEST_POOLS
        .scope(
            Pools::new(
                Network {
                    dns,
                    builder: Some(builder),
                },
                POLICY.as_ref().unwrap().loopback,
            ),
            work,
        )
        .await
}

#[cfg(test)]
fn fixture_clients(url: &Url, access: Access) -> Option<Result<Arc<Clients>, Failure>> {
    // Existing adapter fixtures use a local TLS peer or a local CONNECT tunnel.
    struct Fixed(Vec<std::net::SocketAddr>);
    impl Resolve for Fixed {
        fn resolve(&self, _: Name) -> Resolving {
            let answers = self.0.clone();
            Box::pin(async move { Ok(Box::new(answers.into_iter()) as Addrs) })
        }
    }
    let fixture = crate::services::channel_media_service::TEST_PINNED_TRANSPORT
        .try_with(|f| f.clone())
        .ok();
    if let Some(f) = fixture.filter(|f| url.host_str() == Some(&f.host)) {
        let network = Network {
            dns: Arc::new(Fixed(f.addresses)),
            builder: Some(f.builder),
        };
        return Some(Ok(Arc::new(Clients::new(&network, access, None))));
    }
    crate::services::proxy_service::TARGET_HTTP_CLIENT_BUILDER
        .try_with(|builder| {
            let network = Network {
                dns: Arc::new(SystemDns),
                builder: Some(builder.clone()),
            };
            Ok(Arc::new(Clients::new(&network, access, None)))
        })
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_destination_checks_http_literals_before_pool_selection() {
        for base in [
            "http://0.0.0.0",
            "http://[::]",
            "http://[::ffff:169.254.170.2]",
            "http://224.0.0.1",
        ] {
            let url = Url::parse(base).unwrap();
            assert!(for_url(Some(base), base, &url, false).is_err());
        }
        let base = "https://10.1.2.3:8443";
        let url = Url::parse(base).unwrap();
        assert!(for_url(None, base, &url, false).is_err());
        assert!(for_url(Some(base), base, &url, false).is_ok());
        assert!(
            for_url(
                Some(base),
                base,
                &Url::parse("https://10.1.2.3:8444").unwrap(),
                false
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn proxy_destination_pool_selection_uses_live_exact_approval() {
        with_test_pools(
            Arc::new(SystemDns),
            Arc::new(|| Client::builder().no_proxy()),
            async {
                let base = "https://catalog.internal/api";
                let operation = Url::parse("https://catalog.internal/api/operation").unwrap();
                let public = for_url(None, base, &operation, false).unwrap();
                let approved = for_url(Some(base), base, &operation, false).unwrap();
                assert!(!Arc::ptr_eq(&public, &approved));
                assert!(Arc::ptr_eq(
                    &approved,
                    &for_url(Some(base), base, &operation, false).unwrap()
                ));
                for (effective, host) in [
                    ("https://catalog.internal/v2", "https://catalog.internal"),
                    (base, "https://catalog.internal:8443"),
                    (base, "http://catalog.internal"),
                    (base, "https://another.internal"),
                ] {
                    let request = Url::parse(host).unwrap();
                    assert!(Arc::ptr_eq(
                        &public,
                        &for_url(Some(base), effective, &request, false).unwrap()
                    ));
                }
                assert!(Arc::ptr_eq(
                    &public,
                    &for_url(None, base, &operation, false).unwrap()
                ));
                for n in 0..MAX_PRIVATE_POOLS {
                    let other = format!("https://catalog-{n}.internal");
                    for_url(Some(&other), &other, &Url::parse(&other).unwrap(), false).unwrap();
                }
                assert!(!Arc::ptr_eq(
                    &approved,
                    &for_url(Some(base), base, &operation, false).unwrap()
                ));
            },
        )
        .await;
    }
}

//! Resolve, authorize and pin each direct connection to its effective catalog destination.
pub(crate) mod http;

use std::net::{IpAddr, SocketAddr};
use std::sync::LazyLock;
use std::time::Duration;

use url::Url;

use super::url_validation::{AddressClass, address_class, is_cloud_metadata_host};
use crate::errors::{AppError, AppResult};

tokio::task_local! {
    pub(crate) static INTERNAL_CALLER: &'static str;
}

#[derive(Default)]
struct Policy {
    loopback: bool,
}

impl Policy {
    fn parse(environment: &str, loopback: &str) -> Result<Self, &'static str> {
        let loopback: bool = loopback
            .parse()
            .map_err(|_| "NYXID_PROXY_ALLOW_DEV_LOOPBACK must be true or false")?;
        if loopback && environment.trim().eq_ignore_ascii_case("production") {
            return Err("NYXID_PROXY_ALLOW_DEV_LOOPBACK cannot be enabled in production");
        }
        Ok(Self { loopback })
    }
}

static POLICY: LazyLock<Result<Policy, &'static str>> = LazyLock::new(|| {
    // The backend unit-test executable hosts its own mock HTTP and WS peers.
    // Startup configuration in shipped binaries always defaults to public only.
    let setting = |key, default: &str| match std::env::var(key) {
        Ok(value) => Ok(value),
        Err(std::env::VarError::NotPresent) => Ok(default.to_owned()),
        Err(std::env::VarError::NotUnicode(_)) => Err("Invalid proxy destination configuration"),
    };
    let environment = setting("ENVIRONMENT", "development")?;
    let default_loopback = if cfg!(test) && !environment.trim().eq_ignore_ascii_case("production") {
        "true"
    } else {
        "false"
    };
    Policy::parse(
        &environment,
        &setting("NYXID_PROXY_ALLOW_DEV_LOOPBACK", default_loopback)?,
    )
});

pub(crate) fn validate_configuration() -> Result<(), &'static str> {
    http::initialize()
}

pub(super) fn refused() -> AppError {
    AppError::ValidationError("Direct proxy destination is not permitted".into())
}

/// Bounded facts from the destination check. Resolver errors can contain hostnames.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Failure {
    #[error("url_shape")]
    UrlShape,
    #[error("address_not_permitted")]
    AddressNotPermitted,
    #[error("resolution_unavailable")]
    ResolutionUnavailable,
    #[error("transport_unavailable")]
    TransportUnavailable,
}

impl From<Failure> for AppError {
    fn from(failure: Failure) -> Self {
        match failure {
            Failure::ResolutionUnavailable => Self::PoolAttemptTransport(
                crate::errors::PoolAttemptTransportKind::ResolutionUnavailable,
            ),
            Failure::TransportUnavailable => {
                Self::Internal("Private catalog transport unavailable".into())
            }
            Failure::AddressNotPermitted | Failure::UrlShape => refused(),
        }
    }
}

impl Failure {
    pub(super) fn log(
        self,
        service: &crate::models::downstream_service::DownstreamService,
        method: &str,
        path: &str,
        redirect: bool,
    ) {
        use super::proxy_authorization::{CanonicalPath, rule_matches};
        let path = path
            .split(['?', '#'])
            .next()
            .unwrap_or_default()
            .trim_start_matches('/');
        // The logical route can itself contain opaque user data. Emit only route templates.
        let path = if path.is_empty() {
            "/"
        } else {
            service
                .proxy_operation_policy
                .as_ref()
                .and_then(|policy| {
                    let canonical = CanonicalPath::from_mcp_built(path).ok()?;
                    policy
                        .rules
                        .iter()
                        .find(|rule| rule_matches(rule, method, &canonical))
                        .map(|rule| rule.path_template.as_str())
                })
                .unwrap_or("/{path}")
        };
        let caller = INTERNAL_CALLER.try_with(|caller| *caller).ok();
        match self {
            Self::ResolutionUnavailable | Self::TransportUnavailable => {
                tracing::warn!(service_id=%service.id, reason=%self, method, path, redirect, caller, "Direct proxy destination unavailable")
            }
            Self::AddressNotPermitted | Self::UrlShape => {
                tracing::warn!(service_id=%service.id, reason=%self, method, path, redirect, caller, "Direct proxy destination refused")
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Access {
    private: bool,
    loopback: bool,
}

impl Access {
    pub(crate) fn for_request(
        approved: Option<&str>,
        base: &str,
        url: &Url,
        hosted: bool,
    ) -> Result<Self, Failure> {
        let policy = POLICY.as_ref().map_err(|_| Failure::UrlShape)?;
        let private = approved == Some(base)
            && Url::parse(base).is_ok_and(|base| {
                let scheme = match url.scheme() {
                    "ws" => "http",
                    "wss" => "https",
                    other => other,
                };
                scheme == base.scheme()
                    && base.host() == url.host()
                    && base.port_or_known_default() == url.port_or_known_default()
            });
        Ok(Self {
            private,
            loopback: !hosted && policy.loopback,
        })
    }

    fn permits(self, ip: IpAddr) -> bool {
        match address_class(ip) {
            AddressClass::Public => true,
            AddressClass::Private => self.private,
            AddressClass::Loopback => self.loopback,
            AddressClass::Reserved => false,
        }
    }
    fn permits_all(self, addresses: &[SocketAddr]) -> bool {
        !addresses.is_empty() && addresses.iter().all(|address| self.permits(address.ip()))
    }

    fn check_url(self, url: &Url) -> Result<(), Failure> {
        if !matches!(url.scheme(), "http" | "https" | "ws" | "wss")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(Failure::UrlShape);
        }
        let host = url.host_str().ok_or(Failure::UrlShape)?;
        if is_cloud_metadata_host(host)
            || (!self.loopback && host.trim_end_matches('.').eq_ignore_ascii_case("localhost"))
        {
            return Err(Failure::AddressNotPermitted);
        }
        match url.host().ok_or(Failure::UrlShape)? {
            url::Host::Ipv4(ip) if !self.permits(ip.into()) => Err(Failure::AddressNotPermitted),
            url::Host::Ipv6(ip) if !self.permits(ip.into()) => Err(Failure::AddressNotPermitted),
            _ => Ok(()),
        }
    }
}

pub(crate) struct Destination {
    addresses: Vec<SocketAddr>,
}

impl Destination {
    pub(crate) async fn for_request(
        approved: Option<&str>,
        base: &str,
        request: &str,
    ) -> Result<Self, Failure> {
        let url = Url::parse(request).map_err(|_| Failure::UrlShape)?;
        let access = Access::for_request(approved, base, &url, false)?;
        Self::resolve(&url, access).await
    }

    pub(crate) async fn resolve(url: &Url, access: Access) -> Result<Self, Failure> {
        access.check_url(url)?;
        let port = url.port_or_known_default().ok_or(Failure::UrlShape)?;
        #[cfg(test)]
        if let Ok(Some(addresses)) =
            super::channel_media_service::TEST_PINNED_TRANSPORT.try_with(|fixture| {
                (fixture.target == url.as_str() && url.host_str() == Some(&fixture.host))
                    .then(|| fixture.addresses.clone())
            })
        {
            return Self::checked(addresses, access);
        }
        #[cfg(test)]
        if matches!(url.host(), Some(url::Host::Domain(_)))
            && super::proxy_service::TARGET_HTTP_CLIENT_BUILDER
                .try_with(|_| ())
                .is_ok()
        {
            return Self::checked(vec![SocketAddr::from(([127, 0, 0, 1], port))], access);
        }

        let addresses: Vec<SocketAddr> = match url.host().ok_or(Failure::UrlShape)? {
            url::Host::Ipv4(ip) => vec![SocketAddr::new(ip.into(), port)],
            url::Host::Ipv6(ip) => vec![SocketAddr::new(ip.into(), port)],
            url::Host::Domain(name) => {
                #[cfg(test)]
                if let Some(lookup) = http::test_lookup(name) {
                    let mut addresses = resolve_answers(lookup, Duration::from_secs(10)).await?;
                    for address in &mut addresses {
                        address.set_port(port);
                    }
                    return Self::checked(addresses, access);
                }
                resolve_answers(
                    tokio::net::lookup_host((name, port)),
                    Duration::from_secs(10),
                )
                .await?
            }
        };
        Self::checked(addresses, access)
    }

    fn checked(addresses: Vec<SocketAddr>, access: Access) -> Result<Self, Failure> {
        if addresses.is_empty() {
            return Err(Failure::ResolutionUnavailable);
        }
        if !access.permits_all(&addresses) {
            return Err(Failure::AddressNotPermitted);
        }
        Ok(Self { addresses })
    }

    pub(crate) async fn connect(&self) -> std::io::Result<tokio::net::TcpStream> {
        tokio::net::TcpStream::connect(self.addresses.as_slice()).await
    }
}

async fn resolve_answers<I: Iterator<Item = SocketAddr>, E>(
    lookup: impl Future<Output = Result<I, E>>,
    timeout: Duration,
) -> Result<Vec<SocketAddr>, Failure> {
    let answers = tokio::time::timeout(timeout, lookup)
        .await
        .map_err(|_| Failure::ResolutionUnavailable)?
        .map_err(|_| Failure::ResolutionUnavailable)?;
    Ok(answers.collect())
}

pub(crate) fn approvable_url(raw: &str) -> bool {
    Url::parse(raw).is_ok_and(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url
                .host_str()
                .is_some_and(|host| !is_cloud_metadata_host(host))
            && match url.host() {
                Some(url::Host::Ipv4(ip)) => address_class(ip.into()) != AddressClass::Reserved,
                Some(url::Host::Ipv6(ip)) => address_class(ip.into()) != AddressClass::Reserved,
                Some(url::Host::Domain(_)) => true,
                None => false,
            }
    })
}

fn statically_private_base(raw: &str) -> bool {
    if !approvable_url(raw) {
        return false;
    }
    Url::parse(raw).is_ok_and(|url| match url.host() {
        Some(url::Host::Domain(host)) => {
            let host = host.trim_end_matches('.');
            !host.contains('.')
                || [".svc", ".cluster.local", ".internal", ".local"]
                    .iter()
                    .any(|suffix| host.ends_with(suffix))
        }
        Some(url::Host::Ipv4(ip)) => matches!(
            address_class(ip.into()),
            AddressClass::Private | AddressClass::Loopback
        ),
        Some(url::Host::Ipv6(ip)) => matches!(
            address_class(ip.into()),
            AddressClass::Private | AddressClass::Loopback
        ),
        None => false,
    })
}

const MIGRATION: &str = "proxy_private_catalog_v1";

/// Take a snapshot before serving. Approvals, audits and completion share a transaction.
pub(crate) async fn approve_existing_catalog(db: &mongodb::Database, key: &[u8]) -> AppResult<()> {
    use mongodb::bson::{Document, doc};
    let markers = db.collection::<Document>("schema_migrations");
    if markers
        .find_one(doc! {"_id": MIGRATION, "completed_at": {"$exists":true}})
        .await?
        .is_some()
    {
        return Ok(());
    }
    // Updating an existing marker serializes concurrent startups on the first transaction write.
    markers
        .update_one(
            doc! {"_id": MIGRATION},
            doc! {"$setOnInsert":{"started_at":mongodb::bson::DateTime::now()}},
        )
        .upsert(true)
        .await?;
    let approved =
        super::transaction_service::run_db_only(db, &(db, key), |session, &(db, key)| {
            Box::pin(approve_catalog_snapshot(db, session, key))
        })
        .await?;
    for id in approved {
        tracing::info!(service_id=%id, "Approved existing private catalog destination");
    }
    Ok(())
}

async fn approve_catalog_snapshot(
    db: &mongodb::Database,
    session: &mut mongodb::ClientSession,
    key: &[u8],
) -> AppResult<Vec<String>> {
    use crate::models::{downstream_service, provider_config};
    use mongodb::bson::{Document, doc};
    let claimed = db
        .collection::<Document>("schema_migrations")
        .update_one(
            doc! {"_id":MIGRATION, "completed_at":{"$exists":false}},
            doc! {"$set":{"completed_at":mongodb::bson::DateTime::now()}},
        )
        .session(&mut *session)
        .await?;
    let mut approved = Vec::new();
    if claimed.matched_count == 0 {
        return Ok(approved);
    }
    let rows = db.collection::<Document>(downstream_service::COLLECTION_NAME);
    let mut cursor = rows
        .find(doc! {
            "is_active": true, "catalog_manager_created": {"$ne":true}, "owner_user_id": null,
            "offering_kind": {"$ne":"tool"}, "approved_private_base_url": {"$exists":false}
        })
        .projection(doc! {"base_url":1,"provider_config_id":1})
        .session(&mut *session)
        .await?;
    while let Some(row) = cursor.next(&mut *session).await.transpose()? {
        let (Ok(id), Ok(base)) = (row.get_str("_id"), row.get_str("base_url")) else {
            continue;
        };
        if !statically_private_base(base) {
            continue;
        }
        if let Ok(provider) = row.get_str("provider_config_id") {
            let owned = db
                .collection::<Document>(provider_config::COLLECTION_NAME)
                .find_one(doc! {"_id":provider, "requires_gateway_url":{"$ne":true}})
                .session(&mut *session)
                .await?
                .is_some();
            if !owned {
                continue;
            }
        }
        rows.update_one(
            doc! {"_id":id, "base_url":base},
            doc! {"$set":{"approved_private_base_url":base}},
        )
        .session(&mut *session)
        .await?;
        super::audit_service::log_system_event_in_session(
            db,
            session,
            key,
            "catalog_private_destination_changed",
            serde_json::json!({"service_id":id,"approved":true,"source":"startup"}),
        )
        .await?;
        approved.push(id.to_owned());
    }
    Ok(approved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_access_checks_the_entire_answer() {
        let access = Access {
            private: true,
            loopback: false,
        };
        let addresses: Vec<SocketAddr> = ["10.0.0.1:443", "93.184.216.34:443"]
            .iter()
            .map(|address| address.parse().unwrap())
            .collect();
        assert!(Destination::checked(addresses.clone(), Access::default()).is_err());
        assert!(Destination::checked(addresses.clone(), access).is_ok());
        let mut mixed = addresses;
        mixed.push("169.254.170.2:443".parse().unwrap());
        assert!(Destination::checked(mixed, access).is_err());
        assert!(Destination::checked(vec![], access).is_err());
    }

    #[test]
    fn proxy_destination_approved_catalog_respects_loopback_setting() {
        let base = "https://catalog.internal";
        let url = Url::parse(base).unwrap();
        for hosted in [false, true] {
            let access = Access::for_request(Some(base), base, &url, hosted).unwrap();
            assert!(access.permits("10.1.2.3".parse().unwrap()));
            assert_eq!(
                access.permits("127.0.0.1".parse().unwrap()),
                POLICY.as_ref().unwrap().loopback && !hosted
            );
        }
    }

    #[test]
    fn proxy_destination_approval_is_bound_to_the_effective_base() {
        let base = "https://catalog.internal/api";
        let url = Url::parse("wss://catalog.internal/api/socket").unwrap();
        assert!(
            Access::for_request(Some(base), base, &url, true)
                .unwrap()
                .private
        );
        for changed in [
            "https://catalog.internal/other",
            "https://catalog.internal:8443/api",
        ] {
            assert!(
                !Access::for_request(Some(base), changed, &url, true)
                    .unwrap()
                    .private
            );
        }
        let other = Url::parse("https://other.internal/api").unwrap();
        assert!(
            !Access::for_request(Some(base), base, &other, true)
                .unwrap()
                .private
        );
        assert!(!Access::for_request(None, base, &url, true).unwrap().private);
    }

    #[tokio::test]
    async fn proxy_destination_startup_approvals_survive_restart_and_revocation() {
        use mongodb::bson::{Document, doc};
        let db = crate::test_utils::connect_test_database("proxy_destination_startup")
            .await
            .unwrap();
        let rows = db.collection::<Document>(crate::models::downstream_service::COLLECTION_NAME);
        for (id, url, extra) in [
            ("cluster", "https://app.svc.cluster.local/api", doc! {}),
            ("name", "http://worker", doc! {}),
            ("private", "https://10.1.2.3", doc! {}),
            ("loopback", "http://[::1]", doc! {}),
            ("mapped", "http://[::ffff:10.2.3.4]", doc! {}),
            ("inactive", "http://app.local", doc! {"is_active":false}),
            (
                "tool",
                "http://tool.internal",
                doc! {"offering_kind":"tool"},
            ),
            (
                "manager",
                "http://shared.svc",
                doc! {"catalog_manager_created":true},
            ),
            (
                "custom",
                "http://custom.svc",
                doc! {"owner_user_id":"owner"},
            ),
            ("metadata", "http://metadata.google.internal", doc! {}),
            ("link_local", "http://169.254.1.2", doc! {}),
            ("public", "https://www.example.com", doc! {}),
        ] {
            let mut row = doc! {"_id":id, "base_url":url, "is_active":true};
            row.extend(extra);
            rows.insert_one(row).await.unwrap();
        }
        approve_existing_catalog(&db, &[3; 32]).await.unwrap();
        for id in ["cluster", "name", "private", "loopback", "mapped"] {
            let row = rows.find_one(doc! {"_id":id}).await.unwrap().unwrap();
            assert_eq!(
                row.get_str("base_url").unwrap(),
                row.get_str("approved_private_base_url").unwrap()
            );
        }
        assert_eq!(
            rows.count_documents(doc! {"approved_private_base_url":{"$type":"string"}})
                .await
                .unwrap(),
            5
        );
        rows.update_one(
            doc! {"_id":"cluster"},
            doc! {"$unset":{"approved_private_base_url":""}},
        )
        .await
        .unwrap();
        rows.insert_one(doc! {"_id":"later","is_active":true,"base_url":"http://new.local"})
            .await
            .unwrap();
        approve_existing_catalog(&db, &[3; 32]).await.unwrap();
        assert_eq!(
            rows.count_documents(doc! {"approved_private_base_url":{"$type":"string"}})
                .await
                .unwrap(),
            4
        );
        let audits = db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME);
        assert_eq!(audits.count_documents(doc!{"event_type":"catalog_private_destination_changed","entry_hash":{"$exists":true}}).await.unwrap(), 5);
    }

    #[test]
    fn loopback_opt_in_is_development_only() {
        assert!(!Policy::parse("development", "false").unwrap().loopback);
        assert!(Policy::parse("development", "true").unwrap().loopback);
        assert!(Policy::parse("production", "true").is_err());
        assert!(Policy::parse("production", "yes").is_err());
    }
}

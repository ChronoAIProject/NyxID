//! Bounded ResourceQuota diagnostics. These are observations, never reservations
//! or claims of schedulable cluster capacity. Quotas overlap conjunctively.
use super::remote_machine_kubernetes_identity::KubernetesWorkloadVerifier;
use crate::models::remote_machine::MachineProfileSpec;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeSet, time::Duration};

const SCALE: i128 = 1_000_000_000;
const MAX_QUOTAS: usize = 1000;

/// Exact nonnegative Quantity in nanounits. Unsupported subnano precision and
/// overflow are unknown, not floating point approximations or rounded capacity.
fn quantity(text: &str) -> Option<i128> {
    if text.is_empty() || text.len() > 96 {
        return None;
    }
    let text = text.strip_prefix('+').unwrap_or(text);
    let split = text
        .find(|c: char| !c.is_ascii_digit() && c != '.')
        .unwrap_or(text.len());
    let (number, suffix) = text.split_at(split);
    let (whole, fraction) = number.split_once('.').unwrap_or((number, ""));
    if (whole.is_empty() && fraction.is_empty()) || fraction.contains('.') {
        return None;
    }
    let digits = format!("{whole}{fraction}");
    let value = digits.parse::<i128>().ok()?;
    let (base, power): (i128, i32) = match suffix {
        "" => (10, 0),
        "n" => (10, -9),
        "u" => (10, -6),
        "m" => (10, -3),
        "k" | "K" => (10, 3),
        "M" => (10, 6),
        "G" => (10, 9),
        "T" => (10, 12),
        "P" => (10, 15),
        "E" => (10, 18),
        "Ki" => (2, 10),
        "Mi" => (2, 20),
        "Gi" => (2, 30),
        "Ti" => (2, 40),
        "Pi" => (2, 50),
        "Ei" => (2, 60),
        exponent if exponent.starts_with(['e', 'E']) => (10, exponent[1..].parse().ok()?),
        _ => return None,
    };
    if !(-38..=38).contains(&power) && base == 10 {
        return None;
    }
    let decimal = 9_i32.checked_sub(i32::try_from(fraction.len()).ok()?)?;
    let decimal = if base == 10 {
        decimal.checked_add(power)?
    } else {
        decimal
    };
    let value = if base == 2 {
        value.checked_mul(2_i128.checked_pow(power as u32)?)?
    } else {
        value
    };
    if decimal >= 0 {
        value.checked_mul(10_i128.checked_pow(decimal as u32)?)
    } else {
        let divisor = 10_i128.checked_pow(decimal.unsigned_abs())?;
        (value % divisor == 0).then_some(value / divisor)
    }
}

fn decimal(value: i128) -> String {
    let whole = value / SCALE;
    let fraction = value % SCALE;
    if fraction == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{:09}", fraction)
            .trim_end_matches('0')
            .to_owned()
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Create,
    RetainedVolumeResume,
}

#[derive(Debug, Serialize)]
pub struct Constraint {
    pub resource: String,
    pub hard: Option<String>,
    pub used: Option<String>,
    pub remaining: Option<String>,
    pub deficit: Option<String>,
    pub demand: Option<String>,
    pub additional_profiles_upper_bound: Option<u64>,
    pub reason: Option<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct QuotaRow {
    pub name: String,
    pub applicability: &'static str,
    pub constraints: Vec<Constraint>,
    pub reasons: Vec<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct Inventory {
    pub profile_version: String,
    pub rows: Vec<QuotaRow>,
    pub complete: bool,
    pub observed_at: DateTime<Utc>,
    pub reason: Option<&'static str>,
    pub quota_usage_freshness: &'static str,
}
#[derive(Debug, Serialize)]
pub struct OwnerQuotaView {
    pub status: &'static str,
    pub reason: &'static str,
    pub operation: Operation,
    pub profile_version: String,
    pub additional_profiles_upper_bound: Option<u64>,
    pub observed_at: DateTime<Utc>,
    pub valid_until: DateTime<Utc>,
    pub capacity_guarantee: bool,
    pub quota_usage_freshness: &'static str,
}

fn demand(
    resource: &str,
    operation: Operation,
    storage_class: Option<&str>,
    profile: &MachineProfileSpec,
) -> Option<i128> {
    let create = matches!(operation, Operation::Create);
    let value = match resource {
        "pods" | "count/pods" => 1,
        "count/sandboxes.agents.x-k8s.io"
        | "persistentvolumeclaims"
        | "count/persistentvolumeclaims" => i128::from(create),
        "cpu" | "requests.cpu" | "limits.cpu" => {
            return i128::from(profile.cpu_millis)
                .checked_mul(SCALE)?
                .checked_div(1000);
        }
        "memory" | "requests.memory" | "limits.memory" => {
            i128::from(profile.memory_mib) * 1024 * 1024
        }
        // Namespace mutation and local disk pressure are deployment facts. An
        // undeclared temporary allocation is neither a 2Gi demand nor infinity.
        "ephemeral-storage" | "requests.ephemeral-storage" | "limits.ephemeral-storage" => {
            return None;
        }
        "requests.storage" => {
            if create {
                i128::from(profile.workspace_gib) * 1024 * 1024 * 1024
            } else {
                0
            }
        }
        "services"
        | "count/services"
        | "services.nodeports"
        | "services.loadbalancers"
        | "secrets"
        | "configmaps"
        | "count/secrets"
        | "count/configmaps" => 0,
        key if key.contains(".storageclass.storage.k8s.io/") => {
            let (class, key) = key.split_once(".storageclass.storage.k8s.io/")?;
            match storage_class {
                Some(selected) if selected != class => 0,
                None => return None,
                _ => match key {
                    "requests.storage" => {
                        if create {
                            i128::from(profile.workspace_gib) * 1024 * 1024 * 1024
                        } else {
                            0
                        }
                    }
                    "persistentvolumeclaims" => i128::from(create),
                    _ => return None,
                },
            }
        }
        _ => return None,
    };
    value.checked_mul(SCALE)
}

#[cfg(test)]
fn project(value: &Value, operation: Operation, storage_class: Option<&str>) -> QuotaRow {
    project_profile(
        value,
        operation,
        storage_class,
        &MachineProfileSpec::fixed("quota"),
    )
}

fn project_profile(
    value: &Value,
    operation: Operation,
    storage_class: Option<&str>,
    profile: &MachineProfileSpec,
) -> QuotaRow {
    let mut row = QuotaRow {
        name: value["metadata"]["name"]
            .as_str()
            .filter(|v| v.len() <= 253)
            .unwrap_or("unknown")
            .into(),
        applicability: "applicable",
        constraints: vec![],
        reasons: vec![],
    };
    // Unsupported selectors might apply. Never silently omit them.
    if value["spec"]["scopes"]
        .as_array()
        .is_some_and(|v| !v.is_empty())
        || !value["spec"]["scopeSelector"].is_null()
    {
        row.applicability = "unknown";
        row.reasons.push("quota_scope_not_supported");
    }
    let (Some(spec), Some(hard), Some(used)) = (
        value["spec"]["hard"].as_object(),
        value["status"]["hard"].as_object(),
        value["status"]["used"].as_object(),
    ) else {
        row.reasons.push("quota_status_missing");
        return row;
    };
    if hard.is_empty() || hard.len() > 128 || spec.len() > 128 || used.len() > 128 {
        row.reasons.push("quota_projection_limit_or_empty");
        return row;
    }
    if spec.len() != hard.len()
        || spec.iter().any(|(key, v)| {
            let a = v.as_str().and_then(quantity);
            let b = hard.get(key).and_then(Value::as_str).and_then(quantity);
            a.is_none() || a != b
        })
    {
        row.reasons.push("quota_spec_status_propagation_unknown");
    }
    for (resource, raw_hard) in hard {
        if resource.len() > 253 {
            row.reasons.push("quota_resource_invalid");
            continue;
        }
        let h = raw_hard.as_str().and_then(quantity);
        let u = used
            .get(resource)
            .and_then(Value::as_str)
            .and_then(quantity);
        let d = demand(resource, operation, storage_class, profile);
        let remaining = h.zip(u).map(|(h, u)| (h - u).max(0));
        let reason = if h.is_none() {
            Some("hard_quantity_invalid_or_unsupported")
        } else if u.is_none() {
            Some("used_quantity_missing_invalid_or_unsupported")
        } else if d.is_none() {
            Some("resource_demand_unknown")
        } else {
            None
        };
        row.constraints.push(Constraint {
            resource: resource.clone(),
            hard: h.map(decimal),
            used: u.map(decimal),
            remaining: remaining.map(decimal),
            deficit: h.zip(u).map(|(h, u)| decimal((u - h).max(0))),
            demand: d.map(decimal),
            additional_profiles_upper_bound: remaining
                .zip(d)
                .and_then(|(r, d)| (d > 0).then(|| u64::try_from(r / d).ok()).flatten()),
            reason,
        });
    }
    row
}

impl Inventory {
    fn unknown(reason: &'static str) -> Self {
        Self {
            profile_version: nyxid_machine::managed::PROFILE.into(),
            rows: vec![],
            complete: false,
            observed_at: Utc::now(),
            reason: Some(reason),
            quota_usage_freshness: "controller_reconciliation_time_unknown",
        }
    }
    pub fn owner(
        &self,
        operation: Operation,
        now: DateTime<Utc>,
        overhead_verified: bool,
    ) -> OwnerQuotaView {
        let stale =
            now < self.observed_at || now - self.observed_at > chrono::Duration::seconds(30);
        let uncertain = !self.complete
            || self.reason.is_some()
            || self.rows.iter().any(|q| {
                q.applicability == "unknown"
                    || !q.reasons.is_empty()
                    || q.constraints.iter().any(|c| c.reason.is_some())
            });
        let bounds = self
            .rows
            .iter()
            .filter(|q| q.applicability == "applicable" && q.reasons.is_empty())
            .flat_map(|q| q.constraints.iter())
            .filter_map(|c| c.additional_profiles_upper_bound);
        let upper = bounds.min();
        let (status, reason) = if stale {
            ("unknown", "quota_observation_stale")
        } else if upper == Some(0) {
            ("insufficient", "applicable_quota_exhausted")
        } else if uncertain {
            (
                "unknown",
                self.reason
                    .unwrap_or("quota_inventory_or_applicability_unknown"),
            )
        } else if !overhead_verified {
            (
                "unknown",
                "runtime_overhead_or_admission_mutation_unverified",
            )
        } else {
            (
                "headroom_observed",
                "quota_headroom_is_not_capacity_or_reservation",
            )
        };
        OwnerQuotaView {
            status,
            reason,
            operation,
            profile_version: self.profile_version.clone(),
            additional_profiles_upper_bound: if stale || uncertain || !overhead_verified {
                None
            } else {
                upper
            },
            observed_at: self.observed_at,
            valid_until: self.observed_at + chrono::Duration::seconds(30),
            capacity_guarantee: false,
            quota_usage_freshness: self.quota_usage_freshness,
        }
    }
}

pub async fn observe(
    verifier: Option<&KubernetesWorkloadVerifier>,
    operation: Operation,
    storage_class: Option<&str>,
) -> Inventory {
    observe_profile(
        verifier,
        operation,
        storage_class,
        &MachineProfileSpec::fixed("quota"),
    )
    .await
}

pub async fn observe_profile(
    verifier: Option<&KubernetesWorkloadVerifier>,
    operation: Operation,
    storage_class: Option<&str>,
    profile: &MachineProfileSpec,
) -> Inventory {
    if super::remote_machine_service::validate_fixed_profile(profile).is_err() {
        let mut result = Inventory::unknown("managed_machine_profile_unavailable");
        result.profile_version = profile.profile_version.clone();
        return result;
    }
    let Some(verifier) = verifier else {
        let mut result = Inventory::unknown("quota_transport_unconfigured");
        result.profile_version = profile.profile_version.clone();
        return result;
    };
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        let mut rows = vec![];
        let mut next = String::new();
        let mut seen = BTreeSet::new();
        let mut version = None;
        loop {
            let mut path = "/api/v1/namespaces/nyxid-machine/resourcequotas?limit=100".to_owned();
            if !next.is_empty() {
                path.push_str("&continue=");
                path.push_str(
                    &url::form_urlencoded::byte_serialize(next.as_bytes()).collect::<String>(),
                );
            }
            let response = verifier
                .read_json(&path)
                .await
                .map_err(|_| "quota_inventory_unavailable_or_denied")?;
            if !response["kind"].is_null() && response["kind"] != "ResourceQuotaList" {
                return Err("quota_inventory_kind_invalid");
            }
            let rv = response["metadata"]["resourceVersion"]
                .as_str()
                .ok_or("quota_inventory_version_missing")?;
            if version.as_ref().is_some_and(|v| v != rv) {
                return Err("quota_inventory_version_changed");
            };
            version = Some(rv.to_owned());
            let items = response["items"]
                .as_array()
                .ok_or("quota_inventory_invalid")?;
            if rows.len() + items.len() > MAX_QUOTAS || items.len() > 100 {
                return Err("quota_inventory_limit");
            };
            for item in items {
                if (!item["kind"].is_null() && item["kind"] != "ResourceQuota")
                    || item["metadata"]["namespace"] != "nyxid-machine"
                {
                    return Err("quota_inventory_identity_invalid");
                };
                rows.push(project_profile(item, operation, storage_class, profile));
            }
            next = response["metadata"]["continue"]
                .as_str()
                .unwrap_or("")
                .to_owned();
            if next.is_empty() {
                break;
            }
            if next.len() > 2048 || !seen.insert(next.clone()) || seen.len() >= 10 {
                return Err("quota_inventory_pagination_incomplete");
            };
        }
        Ok(Inventory {
            profile_version: profile.profile_version.clone(),
            rows,
            complete: true,
            observed_at: Utc::now(),
            reason: None,
            quota_usage_freshness: "controller_reconciliation_time_unknown",
        })
    })
    .await;
    match result {
        Ok(Ok(v)) => v,
        Ok(Err(reason)) => Inventory::unknown(reason),
        Err(_) => Inventory::unknown("quota_inventory_timeout"),
    }
}

pub async fn owner_preflight(operation: Operation, profile: &MachineProfileSpec) -> OwnerQuotaView {
    let verifier = KubernetesWorkloadVerifier::from_environment().ok();
    // A configured string is not verified evidence. For resume, today's
    // default/configuration does not identify the retained PVC's class; keep
    // class-specific demand unknown instead of reporting free headroom.
    let storage = if matches!(operation, Operation::Create) {
        super::remote_machine_manager::verified_storage_class_from_environment()
            .await
            .ok()
    } else {
        None
    };
    observe_profile(verifier.as_ref(), operation, storage.as_deref(), profile)
        .await
        .owner(operation, Utc::now(), false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn profile_binding_quota_uses_validated_resources_and_retains_workspace() {
        let profile = MachineProfileSpec::fixed("retained-image");
        assert_eq!(
            demand("requests.cpu", Operation::Create, None, &profile),
            quantity("2000m")
        );
        assert_eq!(
            demand("requests.memory", Operation::Create, None, &profile),
            quantity("2048Mi")
        );
        assert_eq!(
            demand("requests.storage", Operation::Create, None, &profile),
            quantity("20Gi")
        );
        for resource in [
            "requests.storage",
            "persistentvolumeclaims",
            "fast.storageclass.storage.k8s.io/requests.storage",
        ] {
            assert_eq!(
                demand(
                    resource,
                    Operation::RetainedVolumeResume,
                    Some("fast"),
                    &profile
                ),
                Some(0)
            );
        }
        let mut invalid = profile.clone();
        invalid.workspace_gib = 50;
        let rejected = observe_profile(None, Operation::Create, None, &invalid).await;
        assert_eq!(rejected.reason, Some("managed_machine_profile_unavailable"));
        assert!(!rejected.complete);
    }

    #[tokio::test]
    async fn inventory_follows_all_pages_and_rejects_looping_continuation() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path, query_param},
        };
        let server = MockServer::start().await;
        let directory = tempfile::tempdir().unwrap();
        let token = directory.path().join("token");
        std::fs::write(&token, "local-fixture-token").unwrap();
        let verifier =
            KubernetesWorkloadVerifier::for_test_endpoint(server.uri().parse().unwrap(), token);
        Mock::given(method("GET")).and(path("/api/v1/namespaces/nyxid-machine/resourcequotas")).and(query_param("limit","100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"metadata":{"resourceVersion":"7","continue":"next/page"},"items":[quota(json!({"pods":"9"}),json!({"pods":"1"}))]}))).with_priority(10).mount(&server).await;
        Mock::given(query_param("continue","next/page")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"metadata":{"resourceVersion":"7","continue":""},"items":[quota(json!({"requests.cpu":"2"}),json!({"requests.cpu":"2"}))]}))).with_priority(1).mount(&server).await;
        let result = observe(Some(&verifier), Operation::Create, None).await;
        assert!(result.complete);
        assert_eq!(result.rows.len(), 2);
        assert_eq!(
            result.owner(Operation::Create, Utc::now(), true).status,
            "insufficient"
        );
        server.reset().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"metadata":{"resourceVersion":"7","continue":"same"},"items":[]}),
            ))
            .mount(&server)
            .await;
        let result = observe(Some(&verifier), Operation::Create, None).await;
        assert!(!result.complete);
        assert_eq!(result.reason, Some("quota_inventory_pagination_incomplete"));
    }

    #[tokio::test]
    async fn typed_quota_list_accepts_omitted_item_type_meta() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
        let server = MockServer::start().await;
        let token = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(token.path(), "local-fixture-token").unwrap();
        let verifier = KubernetesWorkloadVerifier::for_test_endpoint(
            server.uri().parse().unwrap(),
            token.path().to_path_buf(),
        );
        let mut q = quota(json!({"pods":"2"}), json!({"pods":"0"}));
        q.as_object_mut().unwrap().remove("kind");
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"kind":"ResourceQuotaList","metadata":{"resourceVersion":"1"},"items":[q]}),
            ))
            .mount(&server)
            .await;
        let inventory = observe(Some(&verifier), Operation::Create, None).await;
        assert!(inventory.complete);
        assert_eq!(inventory.rows.len(), 1);
    }

    #[tokio::test]
    async fn denied_malformed_and_partial_inventory_are_unknown() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
        let server = MockServer::start().await;
        let dir = tempfile::tempdir().unwrap();
        let token = dir.path().join("token");
        std::fs::write(&token, "local-fixture-token").unwrap();
        let verifier =
            KubernetesWorkloadVerifier::for_test_endpoint(server.uri().parse().unwrap(), token);
        for response in [ResponseTemplate::new(403),ResponseTemplate::new(200).set_body_json(json!({"items":[]})),ResponseTemplate::new(200).set_body_json(json!({"metadata":{"resourceVersion":"1"},"items":[{"kind":"ResourceQuota","metadata":{"namespace":"foreign"}}]}))] {
            server.reset().await;Mock::given(method("GET")).respond_with(response).mount(&server).await;
            let inventory=observe(Some(&verifier),Operation::Create,None).await;assert!(!inventory.complete);assert_eq!(inventory.owner(Operation::Create,Utc::now(),true).status,"unknown");
        }
    }
    fn quota(hard: Value, used: Value) -> Value {
        json!({"kind":"ResourceQuota","metadata":{"name":"test","namespace":"nyxid-machine"},"spec":{"hard":hard},"status":{"hard":hard,"used":used}})
    }
    fn inventory(rows: Vec<QuotaRow>) -> Inventory {
        Inventory {
            profile_version: nyxid_machine::managed::PROFILE.into(),
            rows,
            complete: true,
            observed_at: Utc::now(),
            reason: None,
            quota_usage_freshness: "controller_reconciliation_time_unknown",
        }
    }
    #[test]
    fn exact_quantities_and_rejection() {
        for (a, b) in [
            ("2", "2000m"),
            ("2Gi", "2147483648"),
            ("20G", "2e10"),
            ("0.001", "1m"),
            ("1.5Gi", "1610612736"),
            (".5", "500m"),
            ("1E", "1e18"),
        ] {
            assert_eq!(quantity(a), quantity(b), "{a} vs {b}");
            assert!(quantity(a).is_some());
        }
        for v in [
            "-1",
            "NaN",
            "1e99",
            "1.0000000001",
            "1MiB",
            "1..2",
            "",
            "1e-10",
        ] {
            assert!(quantity(v).is_none(), "{v}");
        }
    }
    #[test]
    fn quotas_overlap_take_minimum_and_exact_fit() {
        let a = project(
            &quota(json!({"requests.cpu":"6"}), json!({"requests.cpu":"2"})),
            Operation::Create,
            None,
        );
        let b = project(
            &quota(json!({"pods":"4"}), json!({"pods":"3"})),
            Operation::Create,
            None,
        );
        let i = inventory(vec![a, b]);
        let v = i.owner(Operation::Create, Utc::now(), true);
        assert_eq!(v.additional_profiles_upper_bound, Some(1));
        assert_eq!(v.status, "headroom_observed");
        assert!(!v.capacity_guarantee);
    }
    #[test]
    fn missing_used_scope_and_propagation_are_unknown() {
        let mut q = quota(json!({"pods":"2"}), json!({}));
        let i = inventory(vec![project(&q, Operation::Create, None)]);
        assert_eq!(
            i.owner(Operation::Create, Utc::now(), true).status,
            "unknown"
        );
        q["status"]["used"] = json!({"pods":"2"});
        q["spec"]["scopes"] = json!(["BestEffort"]);
        assert_eq!(
            inventory(vec![project(&q, Operation::Create, None)])
                .owner(Operation::Create, Utc::now(), true)
                .status,
            "unknown"
        );
        q["spec"]["scopes"] = json!([]);
        q["spec"]["hard"] = json!({"pods":"3"});
        assert_eq!(
            inventory(vec![project(&q, Operation::Create, None)])
                .owner(Operation::Create, Utc::now(), true)
                .status,
            "unknown"
        );
    }
    #[test]
    fn over_hard_and_retained_resume_do_not_allocate_another_volume() {
        let q = quota(
            json!({"requests.storage":"20Gi","pods":"5"}),
            json!({"requests.storage":"21Gi","pods":"1"}),
        );
        let create = inventory(vec![project(&q, Operation::Create, None)]);
        assert_eq!(
            create.owner(Operation::Create, Utc::now(), true).status,
            "insufficient"
        );
        let resume = inventory(vec![project(&q, Operation::RetainedVolumeResume, None)]);
        assert_eq!(
            resume
                .owner(Operation::RetainedVolumeResume, Utc::now(), true)
                .additional_profiles_upper_bound,
            Some(4)
        );
    }
    #[test]
    fn temporary_quota_is_unknown_without_erasing_memory_or_claiming_infinite_headroom() {
        let q = quota(
            json!({"requests.memory":"4Gi", "requests.ephemeral-storage":"8Gi", "limits.ephemeral-storage":"8Gi"}),
            json!({"requests.memory":"2Gi", "requests.ephemeral-storage":"0", "limits.ephemeral-storage":"0"}),
        );
        let projected = project(&q, Operation::Create, None);
        let memory = projected
            .constraints
            .iter()
            .find(|c| c.resource == "requests.memory")
            .unwrap();
        assert_eq!(memory.demand.as_deref(), Some("2147483648"));
        assert_eq!(memory.additional_profiles_upper_bound, Some(1));
        for temporary in projected
            .constraints
            .iter()
            .filter(|c| c.resource.ends_with("ephemeral-storage"))
        {
            assert_eq!(temporary.demand, None);
            assert_eq!(temporary.additional_profiles_upper_bound, None);
            assert_eq!(temporary.reason, Some("resource_demand_unknown"));
        }
        let inventory = inventory(vec![projected]);
        let current = inventory.owner(Operation::Create, Utc::now(), true);
        assert_eq!(current.status, "unknown");
        assert_eq!(current.additional_profiles_upper_bound, None);
        assert!(!current.capacity_guarantee);
        assert_eq!(current.profile_version, nyxid_machine::managed::PROFILE);
        let retained = inventory.owner(Operation::RetainedVolumeResume, Utc::now(), true);
        assert_eq!(retained.profile_version, nyxid_machine::managed::PROFILE);
        assert_eq!(retained.additional_profiles_upper_bound, None);
    }
    #[test]
    fn storage_class_unknown_and_stale_never_report_capacity() {
        let q = quota(
            json!({"fast.storageclass.storage.k8s.io/requests.storage":"40Gi"}),
            json!({"fast.storageclass.storage.k8s.io/requests.storage":"0"}),
        );
        assert_eq!(
            inventory(vec![project(&q, Operation::Create, None)])
                .owner(Operation::Create, Utc::now(), true)
                .status,
            "unknown"
        );
        let resumed = inventory(vec![project(&q, Operation::RetainedVolumeResume, None)]);
        assert_eq!(
            resumed
                .owner(Operation::RetainedVolumeResume, Utc::now(), true)
                .status,
            "unknown"
        );
        let i = inventory(vec![project(&q, Operation::Create, Some("fast"))]);
        assert_eq!(
            i.owner(Operation::Create, Utc::now(), true)
                .additional_profiles_upper_bound,
            Some(2)
        );
        assert_eq!(
            i.owner(
                Operation::Create,
                Utc::now() + chrono::Duration::seconds(31),
                true
            )
            .status,
            "unknown"
        );
        assert_eq!(
            i.owner(Operation::Create, Utc::now(), false).status,
            "unknown"
        );
    }
}

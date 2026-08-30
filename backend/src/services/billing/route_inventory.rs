#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BillingIngress {
    LlmGateway,
    LlmProvider,
    Proxy,
    Mcp,
    PlatformOperation,
    SshExec,
    SshTunnel,
    SshWebTerminal,
}

impl BillingIngress {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LlmGateway => "llm_gateway",
            Self::LlmProvider => "llm_provider",
            Self::Proxy => "proxy",
            Self::Mcp => "mcp",
            Self::PlatformOperation => "platform_operation",
            Self::SshExec => "ssh_exec",
            Self::SshTunnel => "ssh_tunnel",
            Self::SshWebTerminal => "ssh_web_terminal",
        }
    }
}

#[cfg(test)]
pub const ALL_BILLING_INGRESSES: &[BillingIngress] = &[
    BillingIngress::LlmGateway,
    BillingIngress::LlmProvider,
    BillingIngress::Proxy,
    BillingIngress::Mcp,
    BillingIngress::PlatformOperation,
    BillingIngress::SshExec,
    BillingIngress::SshTunnel,
    BillingIngress::SshWebTerminal,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BillingRoutePolicy {
    Metered(BillingIngress),
    Exempt(&'static str),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BillingEgressPermit {
    _private: (),
}

pub(crate) fn enforce_billing_egress_classification(
    policy: Option<BillingRoutePolicy>,
    expected: BillingIngress,
) -> crate::errors::AppResult<BillingEgressPermit> {
    match policy {
        Some(BillingRoutePolicy::Metered(actual)) if actual == expected => {
            Ok(BillingEgressPermit { _private: () })
        }
        Some(BillingRoutePolicy::Metered(actual)) => {
            Err(crate::errors::AppError::Internal(format!(
                "billing route classification mismatch: expected {}, found {}",
                expected.as_str(),
                actual.as_str()
            )))
        }
        Some(BillingRoutePolicy::Exempt(_)) => Err(crate::errors::AppError::Internal(
            "billing-exempt route attempted downstream forwarding".to_string(),
        )),
        None => Err(crate::errors::AppError::Internal(
            "downstream forwarding route is missing mandatory billing classification".to_string(),
        )),
    }
}

pub(crate) fn enforce_billing_exempt_egress_classification(
    policy: Option<BillingRoutePolicy>,
) -> crate::errors::AppResult<BillingEgressPermit> {
    match policy {
        Some(BillingRoutePolicy::Exempt(_)) => Ok(BillingEgressPermit { _private: () }),
        Some(BillingRoutePolicy::Metered(actual)) => {
            Err(crate::errors::AppError::Internal(format!(
                "metered billing route {} attempted exempt downstream forwarding",
                actual.as_str()
            )))
        }
        None => Err(crate::errors::AppError::Internal(
            "downstream forwarding route is missing mandatory billing classification".to_string(),
        )),
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BillingRouteSpec {
    pub handler: &'static str,
    pub route: &'static str,
    pub policy: BillingRoutePolicy,
}

// Every entry mirrors the declaration that mounts the route; the coverage
// smoke requires exact Metered/Exempt equality.
#[cfg(test)]
pub const BILLING_ROUTE_INVENTORY: &[BillingRouteSpec] = &[
    BillingRouteSpec {
        handler: "handlers::platform_ops::list_operations",
        route: "/api/v1/platform-ops",
        policy: BillingRoutePolicy::Exempt("control-plane discovery; no downstream request"),
    },
    BillingRouteSpec {
        handler: "handlers::platform_preferences::list_preferences",
        route: "/api/v1/platform-ops/preferences",
        policy: BillingRoutePolicy::Exempt(
            "owner spending preference control plane; no downstream request",
        ),
    },
    BillingRouteSpec {
        handler: "handlers::platform_preferences::update_or_delete_preference",
        route: "/api/v1/platform-ops/preferences/{catalog_service_id}",
        policy: BillingRoutePolicy::Exempt(
            "owner spending preference control plane; no downstream request",
        ),
    },
    BillingRouteSpec {
        handler: "handlers::platform_ops::speak",
        route: "/api/v1/platform-ops/speak",
        policy: BillingRoutePolicy::Metered(BillingIngress::PlatformOperation),
    },
    BillingRouteSpec {
        handler: "handlers::platform_ops::call_and_say",
        route: "/api/v1/platform-ops/call-and-say",
        policy: BillingRoutePolicy::Metered(BillingIngress::PlatformOperation),
    },
    BillingRouteSpec {
        handler: "handlers::platform_ops::flight_search",
        route: "/api/v1/platform-ops/flight-search",
        policy: BillingRoutePolicy::Metered(BillingIngress::PlatformOperation),
    },
    BillingRouteSpec {
        handler: "handlers::llm_gateway::gateway_request",
        route: "/api/v1/llm/gateway/v1/{*path}",
        policy: BillingRoutePolicy::Metered(BillingIngress::LlmGateway),
    },
    BillingRouteSpec {
        handler: "handlers::llm_gateway::llm_proxy_request",
        route: "/api/v1/llm/{provider_slug}/v1/{*path}",
        policy: BillingRoutePolicy::Metered(BillingIngress::LlmProvider),
    },
    BillingRouteSpec {
        handler: "handlers::llm_gateway::llm_status",
        route: "/api/v1/llm/status",
        policy: BillingRoutePolicy::Exempt("control-plane discovery; no downstream request"),
    },
    BillingRouteSpec {
        handler: "handlers::proxy::proxy_request",
        route: "/api/v1/proxy/{service_id}/{*path}",
        policy: BillingRoutePolicy::Metered(BillingIngress::Proxy),
    },
    BillingRouteSpec {
        handler: "handlers::proxy::proxy_request_root",
        route: "/api/v1/proxy/{service_id}",
        policy: BillingRoutePolicy::Metered(BillingIngress::Proxy),
    },
    BillingRouteSpec {
        handler: "handlers::proxy::proxy_request_by_slug",
        route: "/api/v1/proxy/s/{slug}/{*path}",
        policy: BillingRoutePolicy::Metered(BillingIngress::Proxy),
    },
    BillingRouteSpec {
        handler: "handlers::proxy::proxy_request_by_slug_root",
        route: "/api/v1/proxy/s/{slug}",
        policy: BillingRoutePolicy::Metered(BillingIngress::Proxy),
    },
    BillingRouteSpec {
        handler: "handlers::proxy::list_proxy_services",
        route: "/api/v1/proxy/services",
        policy: BillingRoutePolicy::Exempt("control-plane discovery; no downstream request"),
    },
    BillingRouteSpec {
        handler: "handlers::assistant_direct::completions",
        route: "/api/v1/assistant/direct/completions",
        policy: BillingRoutePolicy::Metered(BillingIngress::Proxy),
    },
    BillingRouteSpec {
        handler: "handlers::exact_service_approvals::create_request",
        route: "/api/v1/approvals/exact-service/requests",
        policy: BillingRoutePolicy::Exempt("approval control plane; no downstream request"),
    },
    BillingRouteSpec {
        handler: "handlers::exact_service_approvals::observe_request",
        route: "/api/v1/approvals/exact-service/requests/{request_id}/status",
        policy: BillingRoutePolicy::Exempt("approval observation; no downstream request"),
    },
    BillingRouteSpec {
        handler: "handlers::exact_service_approvals::redeem_request",
        route: "/api/v1/approvals/exact-service/requests/{request_id}/redeem",
        policy: BillingRoutePolicy::Metered(BillingIngress::Mcp),
    },
    BillingRouteSpec {
        handler: "handlers::mcp_transport::mcp_post",
        route: "/mcp (POST)",
        policy: BillingRoutePolicy::Metered(BillingIngress::Mcp),
    },
    BillingRouteSpec {
        handler: "handlers::mcp_transport::mcp_get",
        route: "/mcp (GET)",
        policy: BillingRoutePolicy::Exempt("session transport control; tool dispatch meters POST"),
    },
    BillingRouteSpec {
        handler: "handlers::mcp_transport::mcp_delete",
        route: "/mcp (DELETE)",
        policy: BillingRoutePolicy::Exempt("session teardown; no downstream request"),
    },
    BillingRouteSpec {
        handler: "handlers::ssh_exec::ssh_exec",
        route: "/api/v1/ssh/{service_id}/exec",
        policy: BillingRoutePolicy::Metered(BillingIngress::SshExec),
    },
    BillingRouteSpec {
        handler: "handlers::ssh_tunnel::ssh_tunnel_ws",
        route: "/api/v1/ssh/{service_id}",
        policy: BillingRoutePolicy::Metered(BillingIngress::SshTunnel),
    },
    BillingRouteSpec {
        handler: "handlers::ssh_web_terminal::ssh_web_terminal",
        route: "/api/v1/ssh/{service_id}/terminal",
        policy: BillingRoutePolicy::Metered(BillingIngress::SshWebTerminal),
    },
    BillingRouteSpec {
        handler: "handlers::ssh_tunnel::issue_ssh_certificate",
        route: "/api/v1/ssh/{service_id}/certificate",
        policy: BillingRoutePolicy::Exempt("credential control plane; no downstream SSH session"),
    },
    BillingRouteSpec {
        handler: "handlers::public_proxy::public_proxy_request",
        route: "/public/s/{slug}/{*path}",
        policy: BillingRoutePolicy::Exempt(
            "block-not-meter; billable anonymous services are invalid",
        ),
    },
    BillingRouteSpec {
        handler: "handlers::public_proxy::public_proxy_request_root",
        route: "/public/s/{slug}",
        policy: BillingRoutePolicy::Exempt(
            "block-not-meter; billable anonymous services are invalid",
        ),
    },
    BillingRouteSpec {
        handler: "handlers::public_mcp::public_mcp_post",
        route: "/public/mcp",
        policy: BillingRoutePolicy::Exempt("discovery-only; public tools/call never forwards"),
    },
    BillingRouteSpec {
        handler: "handlers::oracle_tasks::submit_task",
        route: "/api/v1/oracle/pools/{id_or_slug}/tasks",
        policy: BillingRoutePolicy::Exempt("user-supplied browser worker capacity"),
    },
    BillingRouteSpec {
        handler: "handlers::oracle_tasks::attach_conversation",
        route: "/api/v1/oracle/pools/{id_or_slug}/attach",
        policy: BillingRoutePolicy::Exempt("user-supplied browser worker capacity"),
    },
    BillingRouteSpec {
        handler: "handlers::oracle_tasks::extract_url",
        route: "/api/v1/oracle/pools/{id_or_slug}/extract",
        policy: BillingRoutePolicy::Exempt("user-supplied browser worker capacity"),
    },
    BillingRouteSpec {
        handler: "handlers::oracle_tasks::pool_status",
        route: "/api/v1/oracle/pools/{id_or_slug}/status",
        policy: BillingRoutePolicy::Exempt("relay control plane; no downstream dispatch"),
    },
    BillingRouteSpec {
        handler: "handlers::oracle_tasks::get_task",
        route: "/api/v1/oracle/tasks/{task_id}",
        policy: BillingRoutePolicy::Exempt("relay control plane; no downstream dispatch"),
    },
    BillingRouteSpec {
        handler: "handlers::oracle_tasks::cancel_task",
        route: "/api/v1/oracle/tasks/{task_id}/cancel",
        policy: BillingRoutePolicy::Exempt("relay control plane; no downstream dispatch"),
    },
    BillingRouteSpec {
        handler: "handlers::oracle_tasks::list_sessions",
        route: "/api/v1/oracle/sessions",
        policy: BillingRoutePolicy::Exempt("relay control plane; no downstream dispatch"),
    },
    BillingRouteSpec {
        handler: "handlers::oracle_tasks::get_session",
        route: "/api/v1/oracle/sessions/{conversation_id}",
        policy: BillingRoutePolicy::Exempt("relay control plane; no downstream dispatch"),
    },
    BillingRouteSpec {
        handler: "handlers::oracle_tasks::close_session",
        route: "/api/v1/oracle/sessions/{conversation_id}/close",
        policy: BillingRoutePolicy::Exempt("relay control plane; no downstream dispatch"),
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BillingIngress {
    LlmGateway,
    LlmProvider,
    Proxy,
    Mcp,
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
    BillingIngress::SshExec,
    BillingIngress::SshTunnel,
    BillingIngress::SshWebTerminal,
];

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BillingRoutePolicy {
    Metered(BillingIngress),
    Exempt(&'static str),
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BillingRouteSpec {
    pub handler: &'static str,
    pub route: &'static str,
    pub policy: BillingRoutePolicy,
}

// Metered entries mirror the declarations that mount the routes; the coverage
// smoke requires exact equality. Exempt entries document forwarding-adjacent
// control-plane routes.
#[cfg(test)]
pub const BILLING_ROUTE_INVENTORY: &[BillingRouteSpec] = &[
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

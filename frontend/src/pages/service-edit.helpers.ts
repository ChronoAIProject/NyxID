import type { DownstreamService, UpdateServicePayload } from "@/types/api";
import type { UpdateServiceFormData } from "@/schemas/services";
import { changedFields, normalizedSet, sameValue } from "@/lib/form-changes";
import { inferSshAuthMode } from "@/lib/ssh-auth-mode";
import { parseAllowedPrincipals } from "@/lib/ssh";

type ServiceFields = Extract<UpdateServicePayload, { base_url?: string }> &
  Extract<UpdateServicePayload, { ssh_config?: unknown }>;
export type ServiceFormPayload = {
  -readonly [K in keyof ServiceFields]: ServiceFields[K];
};

export function serviceFormValues(
  service: DownstreamService,
): UpdateServiceFormData {
  return {
    inference: service.inference ?? null,
    platform_key: service.platform_key
      ? {
          ...service.platform_key,
          allowed_owner_ids: normalizedSet(
            service.platform_key.allowed_owner_ids,
          ),
        }
      : undefined,
    credential: "",
    byok_pricing: laneValues(service.billing?.byok_pricing),
    platform_key_pricing: laneValues(service.billing?.platform_key_pricing),
    platform_charge_nyxid_credentials_only:
      service.billing?.platform_charge_nyxid_credentials_only ?? false,
    service_type: service.service_type === "ssh" ? "ssh" : "http",
    visibility: service.visibility === "private" ? "private" : "public",
    name: service.name,
    description: service.description ?? "",
    base_url: service.service_type === "http" ? service.base_url : "",
    openapi_spec_url: service.openapi_spec_url ?? service.api_spec_url ?? "",
    asyncapi_spec_url: service.asyncapi_spec_url ?? "",
    identity_propagation_mode:
      (service.identity_propagation_mode as UpdateServiceFormData["identity_propagation_mode"]) ??
      "none",
    identity_include_user_id: service.identity_include_user_id ?? false,
    identity_include_email: service.identity_include_email ?? false,
    identity_include_name: service.identity_include_name ?? false,
    identity_jwt_audience: service.identity_jwt_audience ?? "",
    forward_access_token: service.forward_access_token ?? false,
    inject_delegation_token: service.inject_delegation_token ?? false,
    platform_billable: service.billing?.platform_billable ?? false,
    platform_metric:
      (service.billing
        ?.platform_metric as UpdateServiceFormData["platform_metric"]) ??
      "auto",
    platform_price: service.billing?.platform_pricing?.credits_per_unit ?? "",
    delegation_token_scope: service.delegation_token_scope || "llm:proxy",
    homepage_url: service.homepage_url ?? "",
    repository_url: service.repository_url ?? "",
    issues_url: service.issues_url ?? "",
    auth_notes: service.auth_notes ?? "",
    known_limitations: service.known_limitations ?? "",
    required_permissions: service.required_permissions?.join(", ") ?? "",
    examples_url: service.examples_url ?? "",
    recommended_skills: service.recommended_skills?.join(", ") ?? "",
    developer_app_ids: normalizedSet(service.developer_app_ids ?? []),
    supports_proxy_read: service.capabilities?.supports_proxy_read ?? false,
    supports_proxy_write: service.capabilities?.supports_proxy_write ?? false,
    supports_proxy_binary_upload:
      service.capabilities?.supports_proxy_binary_upload ?? false,
    supports_direct_downstream_auth:
      service.capabilities?.supports_direct_downstream_auth ?? false,
    supports_authoring_via_nyx:
      service.capabilities?.supports_authoring_via_nyx ?? false,
    supports_websocket: service.capabilities?.supports_websocket ?? false,
    supports_streaming: service.capabilities?.supports_streaming ?? false,
    host: service.ssh_config?.host ?? "",
    port: service.ssh_config ? String(service.ssh_config.port) : "22",
    ssh_auth_mode: inferSshAuthMode(
      service.ssh_config?.ssh_auth_mode,
      service.ssh_config?.certificate_auth_enabled,
    ),
    certificate_auth_enabled:
      service.ssh_config?.certificate_auth_enabled ?? false,
    certificate_ttl_minutes: service.ssh_config
      ? String(service.ssh_config.certificate_ttl_minutes)
      : "30",
    allowed_principals: service.ssh_config?.allowed_principals.join(", ") ?? "",
    default_request_headers: service.default_request_headers
      ? service.default_request_headers.map((h) => ({ ...h }))
      : [],
    ws_frame_injections: service.ws_frame_injections
      ? service.ws_frame_injections.map((rule) => ({ ...rule }))
      : [],
  };
}

export function serviceFormPayload(
  data: UpdateServiceFormData,
  service: DownstreamService,
): ServiceFormPayload {
  return service.service_type === "ssh"
    ? {
        name: data.name,
        description: data.description || "",
        visibility: data.visibility,
        ssh_config: {
          ssh_auth_mode: inferSshAuthMode(
            data.ssh_auth_mode,
            data.certificate_auth_enabled,
          ),
          host: (data.host ?? "").trim(),
          port: Number(data.port),
          certificate_auth_enabled:
            inferSshAuthMode(
              data.ssh_auth_mode,
              data.certificate_auth_enabled,
            ) === "cert",
          certificate_ttl_minutes: Number(data.certificate_ttl_minutes || "30"),
          allowed_principals: parseAllowedPrincipals(data.allowed_principals),
        },
      }
    : {
        name: data.name,
        description: data.description || "",
        visibility: data.visibility,
        base_url: data.base_url || "",
        openapi_spec_url: data.openapi_spec_url || "",
        asyncapi_spec_url: data.asyncapi_spec_url || "",
        identity_propagation_mode: data.identity_propagation_mode,
        identity_include_user_id: data.identity_include_user_id,
        identity_include_email: data.identity_include_email,
        identity_include_name: data.identity_include_name,
        identity_jwt_audience: data.identity_jwt_audience || "",
        forward_access_token: data.forward_access_token,
        inject_delegation_token: data.inject_delegation_token,
        delegation_token_scope: data.delegation_token_scope || "",
        homepage_url: data.homepage_url || "",
        repository_url: data.repository_url || "",
        issues_url: data.issues_url || "",
        auth_notes: data.auth_notes || "",
        known_limitations: data.known_limitations || "",
        required_permissions: (data.required_permissions || "")
          .split(/[,\n]/)
          .map((s) => s.trim())
          .filter(Boolean),
        examples_url: data.examples_url || "",
        recommended_skills: (data.recommended_skills || "")
          .split(/[,\n]/)
          .map((s) => s.trim())
          .filter(Boolean),
        developer_app_ids: normalizedSet(data.developer_app_ids ?? []),
        inference: data.inference,
        platform_key: data.platform_key
          ? {
              ...data.platform_key,
              allowed_owner_ids: normalizedSet(
                data.platform_key.allowed_owner_ids,
              ),
            }
          : undefined,
        ...(data.credential?.trim()
          ? { credential: data.credential.trim() }
          : {}),
        capabilities: {
          supports_proxy_read: data.supports_proxy_read ?? false,
          supports_proxy_write: data.supports_proxy_write ?? false,
          supports_proxy_binary_upload:
            data.supports_proxy_binary_upload ?? false,
          supports_direct_downstream_auth:
            data.supports_direct_downstream_auth ?? false,
          supports_authoring_via_nyx: data.supports_authoring_via_nyx ?? false,
          supports_websocket: data.supports_websocket ?? false,
          supports_streaming: data.supports_streaming ?? false,
        },
        // Preserve resale config; the toggle only controls the
        // platform-layer opt-in.
        billing: {
          ...(service?.billing ?? {}),
          byok_pricing: data.byok_pricing,
          platform_key_pricing: data.platform_key_pricing,
          platform_charge_nyxid_credentials_only:
            data.platform_charge_nyxid_credentials_only ?? false,
          platform_billable: data.platform_billable ?? false,
          platform_metric:
            data.platform_metric && data.platform_metric !== "auto"
              ? data.platform_metric
              : undefined,
          platform_pricing: data.platform_price?.trim()
            ? {
                credits_per_unit: data.platform_price.trim(),
                lago_metric_code:
                  service.billing?.platform_pricing?.lago_metric_code ?? "",
                sync_status:
                  service.billing?.platform_pricing?.sync_status ?? "pending",
                sync_error:
                  service.billing?.platform_pricing?.sync_error ?? null,
              }
            : undefined,
        },
        ws_frame_injections: data.ws_frame_injections ?? [],
        default_request_headers: data.default_request_headers?.length
          ? data.default_request_headers
          : null,
      };
}

function laneValues(lane: UpdateServiceFormData["byok_pricing"]) {
  return lane
    ? { metric: lane.metric, credits_per_unit: lane.credits_per_unit }
    : null;
}

export function serviceFormPatch(
  data: UpdateServiceFormData,
  service: DownstreamService,
): ServiceFormPayload {
  const values = serviceFormValues(service);
  const before = serviceFormPayload(values, service);
  const after = serviceFormPayload(data, service);
  const patch = changedFields(before, after);
  if (patch.billing) {
    const lanes = changedFields(
      {
        byok_pricing: values.byok_pricing,
        platform_key_pricing: values.platform_key_pricing,
      },
      {
        byok_pricing: laneValues(data.byok_pricing),
        platform_key_pricing: laneValues(data.platform_key_pricing),
      },
    );
    const legacyChanged = [
      "platform_billable",
      "platform_metric",
      "platform_price",
      "platform_charge_nyxid_credentials_only",
    ].some(
      (key) =>
        !sameValue(
          values[key as keyof typeof values],
          data[key as keyof typeof data],
        ),
    );
    if (legacyChanged) {
      const {
        byok_pricing: _byok,
        platform_key_pricing: _platform,
        ...legacy
      } = after.billing!;
      void _byok;
      void _platform;
      patch.billing = { ...legacy, ...lanes };
    } else if (Object.keys(lanes).length) patch.billing = lanes;
    else delete patch.billing;
  }
  return patch;
}

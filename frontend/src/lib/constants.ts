import type { DownstreamService } from "@/types/api";

export type OAuthScopeRisk = "low" | "medium" | "high";

export interface OAuthScopeMeta {
  readonly title: string;
  readonly description: string;
  readonly risk: OAuthScopeRisk;
}

export const AUTH_TYPE_LABELS: Readonly<Record<string, string>> = {
  none: "No Auth",
  api_key: "API Key",
  oauth2: "OAuth 2.0",
  basic: "Basic Auth",
  bearer: "Bearer Token",
  oidc: "OIDC / SSO",
  header: "API Key",
  query: "Query Param",
};

export const SERVICE_CATEGORY_LABELS: Readonly<Record<string, string>> = {
  provider: "SSO Provider",
  connection: "External Service",
  internal: "Internal Service",
};

export const OAUTH_SCOPE_META: Readonly<Record<string, OAuthScopeMeta>> = {
  openid: {
    title: "Authenticate your identity",
    description: "Allows the app to verify who you are.",
    risk: "low",
  },
  profile: {
    title: "Read basic profile",
    description: "Lets the app read your display name and profile details.",
    risk: "low",
  },
  email: {
    title: "Read your email",
    description: "Lets the app access your email address and verification state.",
    risk: "medium",
  },
  offline_access: {
    title: "Long-lived access",
    description:
      "Lets the app refresh tokens without asking you to log in every time.",
    risk: "high",
  },
};

export function scopeRiskClass(risk: OAuthScopeRisk): string {
  if (risk === "high") return "border-red-500/30 bg-red-500/10 text-red-200";
  if (risk === "medium")
    return "border-yellow-500/30 bg-yellow-500/10 text-yellow-200";
  return "border-emerald-500/30 bg-emerald-500/10 text-emerald-200";
}

export function scopeRiskLabel(risk: OAuthScopeRisk): string {
  if (risk === "high") return "High";
  if (risk === "medium") return "Medium";
  return "Low";
}

export function getAuthTypeLabel(service: DownstreamService): string {
  const key = service.auth_type ?? service.auth_method;
  return AUTH_TYPE_LABELS[key] ?? key;
}

export function isOidcService(service: DownstreamService): boolean {
  return (
    service.auth_method === "oidc" ||
    service.auth_type === "oidc" ||
    service.oauth_client_id !== null
  );
}

export function isConnectable(service: DownstreamService): boolean {
  return (
    service.service_category === "connection" ||
    service.service_category === "internal"
  );
}

export function isProvider(service: DownstreamService): boolean {
  return service.service_category === "provider";
}

export function getCredentialInputType(service: DownstreamService): {
  readonly type: "api_key" | "bearer" | "basic" | "none";
  readonly label: string;
  readonly placeholder: string;
} {
  if (!service.requires_user_credential) {
    return { type: "none", label: "", placeholder: "" };
  }
  const authType = service.auth_type ?? service.auth_method;
  switch (authType) {
    case "api_key":
      return { type: "api_key", label: "API Key", placeholder: "sk-..." };
    case "bearer":
      return {
        type: "bearer",
        label: "Bearer Token",
        placeholder: "eyJ...",
      };
    case "basic":
      return {
        type: "basic",
        label: "Username:Password",
        placeholder: "user:pass",
      };
    case "oauth2":
      return {
        type: "bearer",
        label: "Access Token",
        placeholder: "oauth2 token",
      };
    default:
      return {
        type: "api_key",
        label: "Credential",
        placeholder: "Enter credential",
      };
  }
}

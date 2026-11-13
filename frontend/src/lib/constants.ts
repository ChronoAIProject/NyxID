import { translateUi } from "@/i18n/use-ui-translation";
import type { DownstreamService, ProviderConfig } from "@/types/api";

export type OAuthScopeRisk = "low" | "medium" | "high";

export interface OAuthScopeMeta {
  readonly title: string;
  readonly description: string;
  readonly risk: OAuthScopeRisk;
}

export const AUTH_TYPE_LABELS: Readonly<Record<string, string>> = {
  get none() { return translateUi("No Auth"); },
  get api_key() { return translateUi("API Key"); },
  get oauth2() { return translateUi("OAuth 2.0"); },
  get basic() { return translateUi("Basic Auth"); },
  get bearer() { return translateUi("Bearer Token"); },
  get bot_bearer() { return translateUi("Bot Token (Discord)"); },
  get body() { return translateUi("JSON Body Injection"); },
  get path() { return translateUi("Path Prefix"); },
  get ifttt_webhook() { return translateUi("IFTTT Webhooks"); },
  get ifttt_mcp() { return translateUi("IFTTT OAuth"); },
  get oidc() { return translateUi("OIDC / SSO"); },
  get ssh() { return translateUi("SSH Tunnel"); },
  get header() { return translateUi("API Key"); },
  get query() { return translateUi("Query Param"); },
};

export const SERVICE_TYPE_LABELS: Readonly<Record<string, string>> = {
  get http() { return translateUi("HTTP / API"); },
  get ssh() { return translateUi("SSH"); },
};

export const SERVICE_CATEGORY_LABELS: Readonly<Record<string, string>> = {
  get provider() { return translateUi("SSO Provider"); },
  get connection() { return translateUi("External Service"); },
  get internal() { return translateUi("Internal Service"); },
};

export const VISIBILITY_LABELS: Readonly<Record<string, string>> = {
  get public() { return translateUi("Public"); },
  get private() { return translateUi("Private"); },
};

export const OAUTH_SCOPE_META: Readonly<Record<string, OAuthScopeMeta>> = {
  openid: {
    get title() { return translateUi("Authenticate your identity"); },
    get description() { return translateUi("Allows the app to verify who you are."); },
    risk: "low",
  },
  profile: {
    get title() { return translateUi("Read basic profile"); },
    get description() { return translateUi("Lets the app read your display name and profile details."); },
    risk: "low",
  },
  email: {
    get title() { return translateUi("Read your email"); },
    get description() { return translateUi("Lets the app access your email address and verification state."); },
    risk: "medium",
  },
  roles: {
    get title() { return translateUi("Read your roles and permissions"); },
    get description() { return translateUi("Lets the app see your assigned roles and the permissions they grant."); },
    risk: "medium",
  },
  groups: {
    get title() { return translateUi("Read your groups"); },
    get description() { return translateUi("Lets the app see which groups you belong to."); },
    risk: "medium",
  },
  proxy: {
    get title() { return translateUi("Use your connected services"); },
    get description() { return translateUi("Lets the app send requests through NyxID to the services you authorize below."); },
    risk: "high",
  },
  machine: {
    get title() { return translateUi("Use selected Machines"); },
    get description() { return translateUi("Lets the app use commands and files only on the Machines you explicitly approve. It cannot purchase, pause, resume or delete resources."); },
    risk: "high",
  },
  offline_access: {
    get title() { return translateUi("Long-lived access"); },
    get description() { return translateUi("Lets the app refresh tokens without asking you to log in every time."); },
    risk: "high",
  },
  "urn:nyxid:scope:broker_binding": {
    get title() { return translateUi("Durable broker access"); },
    get description() { return translateUi("Lets the app hold a durable NyxID credential that can act as you."); },
    risk: "high",
  },
};

// Risk/severity pills render as a shared `Badge`, mapped onto semantic tokens
// per DESIGN.md ("Severity / risk pills"): high → destructive, medium → warning,
// low → success. The Badge variants already carry the theme-split (dark tint /
// light solid fill) that the hand-rolled raw-palette classes used to duplicate.
export function scopeRiskBadgeVariant(
  risk: OAuthScopeRisk,
): "destructive" | "warning" | "success" {
  if (risk === "high") return "destructive";
  if (risk === "medium") return "warning";
  return "success";
}

export function scopeRiskLabel(risk: OAuthScopeRisk): string {
  if (risk === "high") return "High";
  if (risk === "medium") return "Medium";
  return "Low";
}

export function getAuthTypeLabel(service: DownstreamService): string {
  if (service.service_type === "ssh") {
    return "SSH Tunnel";
  }
  const key = service.auth_type ?? service.auth_method;
  return AUTH_TYPE_LABELS[key] ?? key;
}

export function isOidcService(service: DownstreamService): boolean {
  if (service.service_type !== "http") {
    return false;
  }
  return (
    service.auth_method === "oidc" ||
    service.auth_type === "oidc" ||
    service.oauth_client_id !== null
  );
}

export function isConnectable(service: DownstreamService): boolean {
  if (service.service_type !== "http") {
    return false;
  }
  return (
    service.service_category === "connection" ||
    service.service_category === "internal"
  );
}

export function isProvider(service: DownstreamService): boolean {
  return service.service_category === "provider";
}

export function needsUserCredentials(provider: ProviderConfig): boolean {
  const mode = provider.credential_mode;
  return mode === "user" || mode === "both";
}

export function canConnectProvider(
  provider: ProviderConfig,
  hasUserCredentials = false,
): boolean {
  if (provider.provider_type === "telegram_widget") {
    return provider.has_oauth_config;
  }
  if (
    provider.provider_type !== "oauth2" &&
    provider.provider_type !== "device_code"
  ) {
    return true;
  }
  const mode = provider.credential_mode;
  if (mode === "user") {
    return hasUserCredentials;
  }
  if (mode === "both") {
    return provider.has_oauth_config || hasUserCredentials;
  }
  return provider.has_oauth_config;
}

export function getProviderConnectLabel(
  provider: ProviderConfig,
  hasUserCredentials = false,
): string {
  if (!canConnectProvider(provider, hasUserCredentials)) {
    return "Setup required";
  }

  if (provider.provider_type === "device_code") return "Connect via OAuth";
  if (provider.provider_type === "telegram_widget") return "Login with Telegram";
  return "Connect";
}

export function getProviderConnectHint(
  provider: ProviderConfig,
  hasUserCredentials = false,
): string | null {
  if (!canConnectProvider(provider, hasUserCredentials)) {
    if (provider.provider_type === "telegram_widget") {
      return translateUi("Admin must configure the Telegram bot username and bot token first.");
    }
    const mode = provider.credential_mode;
    if (mode === "user") {
      return translateUi("Set up your OAuth app credentials first.");
    }
    if (mode === "both" && !hasUserCredentials) {
      return translateUi("Admin credentials not configured. Set up your own OAuth app.");
    }
    return translateUi("Admin must configure OAuth client credentials first.");
  }

  return null;
}

export function getCredentialInputType(service: DownstreamService): {
  readonly type: "api_key" | "bearer" | "basic" | "none";
  readonly label: string;
  readonly placeholder: string;
} {
  if (service.service_type !== "http") {
    return { type: "none", label: "", placeholder: "" };
  }
  if (!service.requires_user_credential) {
    return { type: "none", label: "", placeholder: "" };
  }
  const authType = service.auth_type ?? service.auth_method;
  switch (authType) {
    case "api_key":
      return { type: "api_key", get label() { return translateUi("API Key"); }, placeholder: "sk-..." };
    case "bearer":
      return {
        type: "bearer",
        get label() { return translateUi("Bearer Token"); },
        placeholder: "eyJ...",
      };
    case "basic":
      return {
        type: "basic",
        get label() { return translateUi("Username:Password"); },
        placeholder: "user:pass",
      };
    case "oauth2":
      return {
        type: "bearer",
        get label() { return translateUi("Access Token"); },
        placeholder: "oauth2 token",
      };
    default:
      return {
        type: "api_key",
        get label() { return translateUi("Credential"); },
        placeholder: "Enter credential",
      };
  }
}

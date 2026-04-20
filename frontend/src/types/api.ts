export interface User {
  readonly id: string;
  readonly email: string;
  readonly display_name: string | null;
  readonly avatar_url: string | null;
  readonly email_verified: boolean;
  readonly mfa_enabled: boolean;
  readonly is_admin: boolean;
  readonly is_active: boolean;
  readonly created_at: string;
}

export interface ApiKey {
  readonly id: string;
  readonly name: string;
  readonly description: string | null;
  readonly key_prefix: string;
  readonly scopes: string;
  readonly created_at: string;
  readonly last_used_at: string | null;
  readonly expires_at: string | null;
  readonly is_active: boolean;
  readonly allowed_service_ids: readonly string[];
  readonly allowed_node_ids: readonly string[];
  readonly allow_all_services: boolean;
  readonly allow_all_nodes: boolean;
  readonly allowed_services: readonly AllowedServiceInfo[];
  readonly allowed_nodes: readonly AllowedNodeInfo[];
  readonly platform: string | null;
  readonly callback_url: string | null;
  readonly rate_limit_per_second: number | null;
  readonly rate_limit_burst: number | null;
  readonly bindings_count: number;
  /**
   * Provenance: whether this NyxID API key is owned directly by the
   * caller (Personal) or by an org the caller belongs to. Used by the
   * scope and binding pickers to filter user services to those owned by
   * the same owner. Optional for backward compatibility with backends
   * that pre-date the org model.
   */
  readonly credential_source?: import("@/schemas/orgs").CredentialSource;
}

export interface AllowedServiceInfo {
  readonly id: string;
  readonly slug: string;
  readonly label: string;
  readonly catalog_service_name: string | null;
}

export interface AllowedNodeInfo {
  readonly id: string;
  readonly name: string;
  readonly status: string;
}

export interface ApiKeyCreateResponse {
  readonly id: string;
  readonly name: string;
  readonly full_key: string;
  readonly key_prefix: string;
  readonly scopes: string;
  readonly created_at: string;
}

export interface ApiKeyUsageService {
  readonly service_id: string | null;
  readonly service_slug: string;
  readonly service_label: string;
  readonly request_count: number;
  readonly error_count: number;
}

export interface ApiKeyUsageBucket {
  readonly date: string;
  readonly request_count: number;
  readonly error_count: number;
}

export interface ApiKeyUsage {
  readonly api_key_id: string;
  readonly api_key_name: string;
  readonly platform: string | null;
  readonly request_count: number;
  readonly success_count: number;
  readonly error_count: number;
  readonly error_rate: number;
  readonly last_used_at: string | null;
  readonly prompt_tokens: number;
  readonly completion_tokens: number;
  readonly total_tokens: number;
  readonly reported_cost: number | null;
  readonly top_services: readonly ApiKeyUsageService[];
  readonly daily_buckets: readonly ApiKeyUsageBucket[];
}

export interface ApiKeyUsageListResponse {
  readonly usage: readonly ApiKeyUsage[];
  readonly since: string;
  readonly days: number;
}

export interface OAuthClient {
  readonly id: string;
  readonly client_name: string;
  readonly client_type: "public" | "confidential";
  readonly redirect_uris: readonly string[];
  readonly allowed_scopes: string;
  readonly delegation_scopes: string;
  readonly is_active: boolean;
  readonly client_secret: string | null;
  readonly created_at: string;
}

export interface DownstreamService {
  readonly id: string;
  readonly name: string;
  readonly slug: string;
  readonly description: string | null;
  readonly base_url: string;
  readonly service_type: string;
  readonly visibility: string;
  readonly auth_method: string;
  readonly auth_type: string | null;
  readonly auth_key_name: string;
  readonly is_active: boolean;
  readonly oauth_client_id: string | null;
  readonly openapi_spec_url?: string | null;
  readonly api_spec_url: string | null;
  readonly asyncapi_spec_url?: string | null;
  readonly streaming_supported?: boolean;
  readonly ssh_config?: SshServiceConfig | null;
  readonly service_category: string;
  readonly requires_user_credential: boolean;
  readonly created_by: string;
  readonly created_at: string;
  readonly updated_at: string;
  readonly identity_propagation_mode?: string;
  readonly identity_include_user_id?: boolean;
  readonly identity_include_email?: boolean;
  readonly identity_include_name?: boolean;
  readonly identity_jwt_audience?: string | null;
  readonly forward_access_token?: boolean;
  readonly inject_delegation_token?: boolean;
  readonly delegation_token_scope?: string;
  // Rich metadata for AI agent discovery
  readonly homepage_url?: string | null;
  readonly repository_url?: string | null;
  readonly issues_url?: string | null;
  readonly capabilities?: ServiceCapabilities | null;
  readonly auth_notes?: string | null;
  readonly known_limitations?: string | null;
  readonly required_permissions?: readonly string[] | null;
  readonly examples_url?: string | null;
  readonly recommended_skills?: readonly string[] | null;
  readonly developer_app_ids?: readonly string[] | null;
  /**
   * NyxID#356: admin-configured default HTTP headers injected on every
   * proxied request. `null` / `undefined` means no defaults are set.
   */
  readonly default_request_headers?: readonly DefaultRequestHeader[] | null;
}

export interface DefaultRequestHeader {
  readonly name: string;
  readonly value: string;
  readonly overridable: boolean;
  readonly sensitive: boolean;
}

export interface ServiceCapabilities {
  readonly supports_proxy_read: boolean;
  readonly supports_proxy_write: boolean;
  readonly supports_proxy_binary_upload: boolean;
  readonly supports_direct_downstream_auth: boolean;
  readonly supports_authoring_via_nyx: boolean;
  readonly supports_websocket: boolean;
  readonly supports_streaming: boolean;
}

export interface SshServiceConfig {
  readonly host: string;
  readonly port: number;
  readonly certificate_auth_enabled: boolean;
  readonly certificate_ttl_minutes: number;
  readonly allowed_principals: readonly string[];
  readonly ca_public_key: string | null;
}

export interface SshServiceConfigInput {
  readonly host: string;
  readonly port: number;
  readonly certificate_auth_enabled: boolean;
  readonly certificate_ttl_minutes: number;
  readonly allowed_principals: readonly string[];
}

export type CreateServicePayload =
  | {
      readonly name: string;
      readonly description?: string;
      readonly service_type: "http";
      readonly visibility?: string;
      readonly base_url: string;
      readonly auth_type: string;
      /// Required when auth_type is "body" (e.g. "app_secret" for Lark).
      /// Also accepted as an override for other auth types.
      readonly auth_key_name?: string;
      readonly service_category?: string;
    }
  | {
      readonly name: string;
      readonly description?: string;
      readonly service_type: "ssh";
      readonly visibility?: string;
      readonly service_category?: string;
      readonly ssh_config: SshServiceConfigInput;
    };

export type UpdateServicePayload =
  | {
      readonly name?: string;
      readonly description?: string;
      readonly visibility?: string;
      readonly base_url?: string;
      readonly is_active?: boolean;
      readonly openapi_spec_url?: string;
      readonly asyncapi_spec_url?: string;
      readonly identity_propagation_mode?: string;
      readonly identity_include_user_id?: boolean;
      readonly identity_include_email?: boolean;
      readonly identity_include_name?: boolean;
      readonly identity_jwt_audience?: string;
      readonly forward_access_token?: boolean;
      readonly inject_delegation_token?: boolean;
      readonly delegation_token_scope?: string;
      readonly homepage_url?: string;
      readonly repository_url?: string;
      readonly issues_url?: string;
      readonly capabilities?: ServiceCapabilities;
      readonly auth_notes?: string;
      readonly known_limitations?: string;
      readonly required_permissions?: readonly string[];
      readonly examples_url?: string;
      readonly recommended_skills?: readonly string[];
      readonly developer_app_ids?: readonly string[];
      /**
       * NyxID#356 tri-state update semantics:
       * - `undefined` (omit) leaves the existing value unchanged
       * - `null` clears the list
       * - array replaces with a validated list
       */
      readonly default_request_headers?:
        | null
        | readonly DefaultRequestHeader[];
    }
  | {
      readonly name?: string;
      readonly description?: string;
      readonly visibility?: string;
      readonly is_active?: boolean;
      readonly ssh_config?: SshServiceConfigInput;
    };

export interface ServiceEndpoint {
  readonly id: string;
  readonly service_id: string;
  readonly name: string;
  readonly description: string | null;
  readonly method: string;
  readonly path: string;
  readonly parameters: unknown | null;
  readonly request_body_schema: unknown | null;
  readonly response_description: string | null;
  readonly is_active: boolean;
  readonly created_at: string;
  readonly updated_at: string;
}

export interface DiscoverEndpointsResponse {
  readonly endpoints: readonly ServiceEndpoint[];
  readonly message: string;
}

export interface OidcCredentials {
  readonly client_id: string;
  readonly client_secret: string;
  readonly redirect_uris: readonly string[];
  readonly allowed_scopes: string;
  readonly delegation_scopes: string;
  readonly issuer: string;
  readonly authorization_endpoint: string;
  readonly token_endpoint: string;
  readonly userinfo_endpoint: string;
  readonly jwks_uri: string;
}

export interface RegenerateSecretResponse {
  readonly client_secret: string;
  readonly message: string;
}

export interface RedirectUrisResponse {
  readonly redirect_uris: readonly string[];
}

export interface UserServiceConnection {
  readonly service_id: string;
  readonly service_name: string;
  readonly service_category: string;
  readonly auth_type: string | null;
  readonly has_credential: boolean;
  readonly credential_label: string | null;
  readonly connected_at: string;
}

export interface Session {
  readonly id: string;
  readonly ip_address: string;
  readonly user_agent: string;
  readonly created_at: string;
  readonly expires_at: string;
}

export interface AuditLogEntry {
  readonly id: string;
  readonly action: string;
  readonly ip_address: string;
  readonly details: string | null;
  readonly created_at: string;
}

export interface MfaSetupResponse {
  readonly factor_id: string;
  readonly secret: string;
  readonly qr_code_url: string;
}

export interface MfaConfirmResponse {
  readonly message: string;
  readonly recovery_codes: readonly string[];
}

export interface ApiErrorResponse {
  readonly error: string;
  readonly error_code: number;
  readonly message: string;
  readonly consent_url?: string;
  readonly request_id?: string;
  readonly approve_url?: string;
}

export interface LoginCredentials {
  readonly email: string;
  readonly password: string;
}

export interface RegisterCredentials {
  readonly email: string;
  readonly password: string;
  readonly display_name: string;
}

export interface LoginResponse {
  readonly user_id: string;
  readonly access_token?: string;
  readonly expires_in?: number;
  readonly refresh_token?: string;
}

export interface RegisterResponse {
  readonly user_id: string;
  readonly message: string;
}

export interface MfaRequiredError {
  readonly error: string;
  readonly error_code: number;
  readonly message: string;
  readonly session_token: string;
}

export interface MfaVerifyRequest {
  readonly code: string;
  readonly mfa_token: string;
}

export interface PublicConfig {
  readonly mcp_url: string;
  readonly node_ws_url: string;
  readonly version: string;
  readonly social_providers: readonly string[];
  readonly invite_code_required: boolean;
  readonly email_auth_enabled: boolean;
}

export type CredentialMode = "admin" | "user" | "both";

export interface ProviderConfig {
  readonly id: string;
  readonly slug: string;
  readonly name: string;
  readonly description: string | null;
  readonly provider_type: "oauth2" | "api_key" | "device_code" | "telegram_widget";
  readonly has_oauth_config: boolean;
  readonly credential_mode: CredentialMode;
  readonly default_scopes: readonly string[] | null;
  readonly supports_pkce: boolean;
  readonly device_code_url: string | null;
  readonly device_token_url: string | null;
  readonly device_verification_url: string | null;
  readonly hosted_callback_url: string | null;
  readonly api_key_instructions: string | null;
  readonly api_key_url: string | null;
  readonly token_endpoint_auth_method: string;
  readonly extra_auth_params: Readonly<Record<string, string>> | null;
  readonly device_code_format: string;
  readonly client_id_param_name: string | null;
  readonly requires_gateway_url: boolean;
  readonly icon_url: string | null;
  readonly documentation_url: string | null;
  readonly is_active: boolean;
  readonly created_at: string;
  readonly updated_at: string;
}

export interface UserProviderCredentials {
  readonly provider_config_id: string;
  readonly has_credentials: boolean;
  readonly label: string | null;
  readonly created_at: string | null;
  readonly updated_at: string | null;
}

export interface UserProviderToken {
  readonly provider_id: string;
  readonly provider_name: string;
  readonly provider_slug: string;
  readonly provider_type: string;
  readonly status: "active" | "expired" | "revoked" | "refresh_failed";
  readonly label: string | null;
  readonly gateway_url: string | null;
  readonly expires_at: string | null;
  readonly last_used_at: string | null;
  readonly connected_at: string;
  readonly metadata?: Readonly<Record<string, string>> | null;
}

export interface OAuthInitiateResponse {
  readonly authorization_url: string;
}

export interface DeviceCodeInitiateResponse {
  readonly user_code: string;
  readonly verification_uri: string;
  readonly state: string;
  readonly expires_in: number;
  readonly interval: number;
}

export interface DeviceCodePollRequest {
  readonly state: string;
}

export interface DeviceCodePollResponse {
  readonly status: "pending" | "slow_down" | "expired" | "denied" | "complete";
  readonly interval?: number;
}

export interface ServiceProviderRequirement {
  readonly id: string;
  readonly service_id: string;
  readonly provider_config_id: string;
  readonly provider_name: string;
  readonly provider_slug: string;
  readonly required: boolean;
  readonly scopes: readonly string[] | null;
  readonly injection_method: string;
  readonly injection_key: string | null;
}

export interface UserTokenListResponse {
  readonly tokens: readonly UserProviderToken[];
}

export interface ProviderListResponse {
  readonly providers: readonly ProviderConfig[];
}

export interface MessageResponse {
  readonly message: string;
}

export interface ProviderActionResponse {
  readonly status: string;
  readonly message: string;
}

export interface LlmProviderStatus {
  readonly provider_slug: string;
  readonly provider_name: string;
  readonly status: "ready" | "not_connected" | "expired";
  readonly proxy_url: string;
}

export interface LlmStatusResponse {
  readonly providers: readonly LlmProviderStatus[];
  readonly gateway_url: string;
  readonly supported_models: readonly string[];
}

/**
 * Social Token Exchange (RFC 8693) - used by mobile apps with native SDK login.
 * Mobile apps exchange provider tokens (Google ID token, GitHub access token) for NyxID token sets
 * via POST /oauth/token with grant_type=urn:ietf:params:oauth:grant-type:token-exchange.
 */

export type SocialTokenExchangeProvider = "google" | "github" | "apple";

export interface SocialTokenExchangeRequest {
  readonly grant_type: "urn:ietf:params:oauth:grant-type:token-exchange";
  readonly subject_token: string;
  readonly subject_token_type:
    | "urn:ietf:params:oauth:token-type:id_token"
    | "urn:ietf:params:oauth:token-type:access_token";
  readonly client_id: string;
  readonly client_secret?: string;
  readonly provider: SocialTokenExchangeProvider;
}

export interface SocialTokenExchangeResponse {
  readonly access_token: string;
  readonly token_type: "Bearer";
  readonly expires_in: number;
  readonly refresh_token: string;
  readonly id_token?: string;
  readonly scope: string;
  readonly issued_token_type: string;
}

export interface TelegramWidgetConfig {
  readonly bot_username: string;
  readonly redirect_url?: string;
}

export interface TelegramLoginData {
  readonly id: number;
  readonly first_name: string;
  readonly last_name?: string;
  readonly username?: string;
  readonly photo_url?: string;
  readonly auth_date: number;
  readonly hash: string;
}

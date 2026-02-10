export interface User {
  readonly id: string;
  readonly email: string;
  readonly name: string | null;
  readonly avatar_url: string | null;
  readonly email_verified: boolean;
  readonly mfa_enabled: boolean;
  readonly created_at: string;
}

export interface ApiKey {
  readonly id: string;
  readonly name: string;
  readonly key_prefix: string;
  readonly scopes: string;
  readonly created_at: string;
  readonly last_used_at: string | null;
  readonly expires_at: string | null;
  readonly revoked: boolean;
}

export interface ApiKeyCreateResponse {
  readonly id: string;
  readonly name: string;
  readonly key: string;
  readonly key_prefix: string;
  readonly scopes: string;
  readonly created_at: string;
  readonly expires_at: string | null;
}

export interface OAuthClient {
  readonly id: string;
  readonly client_name: string;
  readonly client_type: "public" | "confidential";
  readonly redirect_uris: readonly string[];
  readonly allowed_scopes: string;
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
  readonly auth_method: string;
  readonly auth_type: string | null;
  readonly auth_key_name: string;
  readonly is_active: boolean;
  readonly oauth_client_id: string | null;
  readonly api_spec_url: string | null;
  readonly service_category: string;
  readonly requires_user_credential: boolean;
  readonly created_by: string;
  readonly created_at: string;
  readonly updated_at: string;
}

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
  readonly secret: string;
  readonly qr_code_url: string;
  readonly recovery_codes: readonly string[];
}

export interface ApiErrorResponse {
  readonly error: string;
  readonly error_code: string;
  readonly message: string;
}

export interface LoginCredentials {
  readonly email: string;
  readonly password: string;
}

export interface RegisterCredentials {
  readonly email: string;
  readonly password: string;
  readonly name: string;
}

export interface LoginResponse {
  readonly user_id: string;
  readonly access_token: string;
  readonly expires_in: number;
}

export interface RegisterResponse {
  readonly user_id: string;
  readonly message: string;
}

export interface MfaRequiredError {
  readonly error: string;
  readonly error_code: string;
  readonly message: string;
  readonly session_token: string;
}

export interface MfaVerifyRequest {
  readonly code: string;
  readonly mfa_token: string;
}

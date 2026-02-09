export interface User {
  readonly id: string
  readonly email: string
  readonly name: string | null
  readonly avatar_url: string | null
  readonly email_verified: boolean
  readonly mfa_enabled: boolean
  readonly created_at: string
}

export interface ApiKey {
  readonly id: string
  readonly name: string
  readonly key_prefix: string
  readonly scopes: string
  readonly created_at: string
  readonly last_used_at: string | null
  readonly expires_at: string | null
  readonly revoked: boolean
}

export interface ApiKeyCreateResponse {
  readonly id: string
  readonly name: string
  readonly key: string
  readonly key_prefix: string
  readonly scopes: string
  readonly created_at: string
  readonly expires_at: string | null
}

export interface OAuthClient {
  readonly id: string
  readonly name: string
  readonly redirect_uris: readonly string[]
  readonly scopes: readonly string[]
  readonly grant_types: readonly string[]
  readonly client_type: "public" | "confidential"
}

export interface DownstreamService {
  readonly id: string
  readonly name: string
  readonly base_url: string
  readonly auth_type: "api_key" | "oauth2" | "basic" | "bearer"
  readonly created_at: string
}

export interface UserServiceConnection {
  readonly service_id: string
  readonly service_name: string
  readonly connected_at: string
}

export interface Session {
  readonly id: string
  readonly ip_address: string
  readonly user_agent: string
  readonly created_at: string
  readonly expires_at: string
}

export interface AuditLogEntry {
  readonly id: string
  readonly action: string
  readonly ip_address: string
  readonly details: string | null
  readonly created_at: string
}

export interface MfaSetupResponse {
  readonly secret: string
  readonly qr_code_url: string
  readonly recovery_codes: readonly string[]
}

export interface ApiErrorResponse {
  readonly error: string
  readonly error_code: string
  readonly message: string
}

export interface LoginCredentials {
  readonly email: string
  readonly password: string
}

export interface RegisterCredentials {
  readonly email: string
  readonly password: string
  readonly name: string
}

export interface LoginResponse {
  readonly user_id: string
  readonly access_token: string
  readonly expires_in: number
}

export interface RegisterResponse {
  readonly user_id: string
  readonly message: string
}

export interface MfaRequiredError {
  readonly error: string
  readonly error_code: string
  readonly message: string
  readonly session_token: string
}

export interface MfaVerifyRequest {
  readonly code: string
  readonly mfa_token: string
}

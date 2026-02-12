export interface ServiceAccount {
  readonly id: string;
  readonly name: string;
  readonly description: string | null;
  readonly client_id: string;
  readonly secret_prefix: string;
  readonly allowed_scopes: string;
  readonly role_ids: readonly string[];
  readonly is_active: boolean;
  readonly rate_limit_override: number | null;
  readonly created_by: string;
  readonly created_at: string;
  readonly updated_at: string;
  readonly last_authenticated_at: string | null;
}

export interface ServiceAccountListResponse {
  readonly service_accounts: readonly ServiceAccount[];
  readonly total: number;
  readonly page: number;
  readonly per_page: number;
}

export interface CreateServiceAccountRequest {
  readonly name: string;
  readonly description?: string;
  readonly allowed_scopes: string;
  readonly role_ids?: readonly string[];
  readonly rate_limit_override?: number;
}

export interface CreateServiceAccountResponse {
  readonly id: string;
  readonly name: string;
  readonly client_id: string;
  readonly client_secret: string;
  readonly allowed_scopes: string;
  readonly role_ids: readonly string[];
  readonly is_active: boolean;
  readonly created_at: string;
  readonly message: string;
}

export interface UpdateServiceAccountRequest {
  readonly name?: string;
  readonly description?: string;
  readonly allowed_scopes?: string;
  readonly role_ids?: readonly string[];
  readonly rate_limit_override?: number | null;
  readonly is_active?: boolean;
}

export interface RotateSecretResponse {
  readonly client_id: string;
  readonly client_secret: string;
  readonly secret_prefix: string;
  readonly message: string;
}

export interface RevokeTokensResponse {
  readonly revoked_count: number;
  readonly message: string;
}

export interface AdminActionResponse {
  readonly message: string;
}

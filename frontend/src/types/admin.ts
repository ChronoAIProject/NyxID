export interface AdminUser {
  readonly id: string;
  readonly email: string;
  readonly display_name: string | null;
  readonly avatar_url: string | null;
  readonly email_verified: boolean;
  readonly is_active: boolean;
  readonly is_admin: boolean;
  readonly mfa_enabled: boolean;
  readonly role_ids?: readonly string[];
  readonly group_ids?: readonly string[];
  readonly created_at: string;
  readonly last_login_at: string | null;
}

export interface AdminUserListResponse {
  readonly users: readonly AdminUser[];
  readonly total: number;
  readonly page: number;
  readonly per_page: number;
}

export interface AdminSession {
  readonly id: string;
  readonly ip_address: string | null;
  readonly user_agent: string | null;
  readonly created_at: string;
  readonly expires_at: string;
  readonly last_active_at: string;
  readonly revoked: boolean;
}

export interface AdminSessionListResponse {
  readonly sessions: readonly AdminSession[];
  readonly total: number;
}

export interface UpdateUserRequest {
  readonly display_name?: string;
  readonly email?: string;
  readonly avatar_url?: string;
}

export interface SetRoleRequest {
  readonly is_admin: boolean;
}

export interface SetStatusRequest {
  readonly is_active: boolean;
}

export interface AdminActionResponse {
  readonly message: string;
}

export interface RoleUpdateResponse {
  readonly id: string;
  readonly is_admin: boolean;
  readonly message: string;
}

export interface StatusUpdateResponse {
  readonly id: string;
  readonly is_active: boolean;
  readonly message: string;
}

export interface VerifyEmailResponse {
  readonly id: string;
  readonly email_verified: boolean;
  readonly message: string;
}

export interface RevokeSessionsResponse {
  readonly revoked_count: number;
  readonly message: string;
}

export interface CreateUserRequest {
  readonly email: string;
  readonly password: string;
  readonly display_name?: string;
  readonly role: "admin" | "user";
}

export interface CreateUserResponse {
  readonly id: string;
  readonly email: string;
  readonly display_name: string | null;
  readonly is_admin: boolean;
  readonly is_active: boolean;
  readonly email_verified: boolean;
  readonly created_at: string;
  readonly message: string;
}

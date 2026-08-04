import type { PlatformRole } from "./api";

export interface AdminUser {
  readonly id: string;
  readonly email: string;
  readonly display_name: string | null;
  readonly avatar_url: string | null;
  readonly email_verified: boolean;
  readonly is_active: boolean;
  readonly is_admin: boolean;
  /// Read-only platform admin (issue #715). Older backends omit this field.
  readonly is_operator?: boolean;
  /// Resolved platform role string. Older backends omit this; callers should
  /// fall back to deriving from `is_admin` / `is_operator`.
  readonly role?: PlatformRole;
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

/// Body for `PATCH /admin/users/{id}/role`. Use `role` for the three-tier
/// model (admin / operator / user). The legacy `is_admin` field is still
/// accepted by the backend for back-compat but new callers should send
/// `role`.
export interface SetRoleRequest {
  readonly role?: PlatformRole;
  readonly is_admin?: boolean;
}

export interface SetStatusRequest {
  readonly is_active: boolean;
}

export interface AdminActionResponse {
  readonly message: string;
}

export interface RoleUpdateResponse {
  readonly id: string;
  readonly role: PlatformRole;
  readonly is_admin: boolean;
  readonly is_operator: boolean;
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
  readonly role: PlatformRole;
}

export interface CreateUserResponse {
  readonly id: string;
  readonly email: string;
  readonly display_name: string | null;
  readonly role: PlatformRole;
  readonly is_admin: boolean;
  readonly is_operator: boolean;
  readonly is_active: boolean;
  readonly email_verified: boolean;
  readonly created_at: string;
  readonly message: string;
}

export interface AdminAuditLogEntry {
  readonly id: string;
  readonly user_id: string | null;
  readonly api_key_id: string | null;
  readonly api_key_name: string | null;
  readonly event_type: string;
  readonly event_data: Record<string, unknown> | null;
  readonly ip_address: string | null;
  readonly user_agent: string | null;
  readonly created_at: string;
}

export interface AdminAuditLogListResponse {
  readonly entries: readonly AdminAuditLogEntry[];
  readonly total: number;
  readonly page: number;
  readonly per_page: number;
}

// ── Invite codes ──

export interface InviteCodeUsage {
  readonly user_id: string;
  readonly used_at: string;
  /** Email of the redeeming user, or null if the user has been deleted. */
  readonly user_email: string | null;
  /** Display name of the redeeming user, or null if not set / deleted. */
  readonly user_display_name: string | null;
}

/** Resolved creator info for an invite code. Null when the admin who minted
 * the code has been deleted since — callers should fall back to rendering the
 * raw `created_by` UUID in that case. */
export interface InviteCodeCreator {
  /** Email of the admin. Always present whenever the creator object itself is non-null. */
  readonly email: string;
  /** Display name of the admin, or null if they have no display name set. */
  readonly display_name: string | null;
}

export interface InviteCode {
  readonly id: string;
  readonly code: string;
  readonly max_uses: number;
  readonly used_count: number;
  /** UUID of the admin who created this code. Stable foreign key. */
  readonly created_by: string;
  /** Resolved creator details (email + display name). Null if the admin has
   * been deleted since the code was minted. */
  readonly creator: InviteCodeCreator | null;
  readonly note: string | null;
  readonly is_active: boolean;
  readonly created_at: string;
  readonly updated_at: string;
  readonly usages: readonly InviteCodeUsage[];
}

export interface InviteCodeListResponse {
  readonly invite_codes: readonly InviteCode[];
}

export interface CreateInviteCodeRequest {
  readonly max_uses?: number;
  readonly note?: string;
}

export interface UpdateInviteCodeRequest {
  /**
   * The new note value. The PATCH endpoint is authoritative — whatever you
   * send (or omit) becomes the stored value. A non-empty string sets the
   * note; `""`, `null`, or omitting the field all clear it. There is no
   * "leave unchanged" mode today, so always send the full intended value.
   */
  readonly note?: string | null;
}

export interface DeactivateInviteCodeResponse {
  readonly message: string;
}

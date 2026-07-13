import type { PlatformRole } from "./api";
import type {
  DataTableFilterField,
  DataTableFilterOperator,
  DataTableFilterOption,
  DataTableFilterSelections,
  DataTableFilterValueType,
  DataTableSearchField,
  DataTableSearchGroup,
} from "./data-table";

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

// ── Admin OAuth clients / broker rollout settings ──

export interface AdminOAuthClient {
  readonly id: string;
  readonly client_name: string;
  readonly client_type: "public" | "confidential" | string;
  readonly created_by: string | null;
  readonly redirect_uris: readonly string[];
  readonly allowed_scopes: string;
  readonly delegation_scopes: string;
  readonly broker_capability_enabled: boolean;
  readonly broker_capability_effective: boolean;
  readonly broker_capability_source: "none" | "flag" | "scope";
  readonly revocation_webhook_url: string | null;
  readonly is_active: boolean;
  readonly client_secret: string | null;
  readonly created_at: string;
}

export type AdminOAuthClientTypeFilter = "public" | "confidential" | "other";
export type AdminOAuthClientCreatorType =
  | "dynamic_registration"
  | "system"
  | "owned"
  | "ownerless";
export type AdminOAuthClientBrokerFilter =
  | "enabled"
  | "disabled"
  | "flag"
  | "scope";
export type AdminOAuthClientSort =
  | "-created_at"
  | "created_at"
  | "client_name"
  | "-client_name"
  | "client_type"
  | "-client_type"
  | "created_by"
  | "-created_by"
  | "broker"
  | "-broker"
  | "allowed_scopes"
  | "-allowed_scopes"
  | "-is_active"
  | "is_active";

export type AdminOAuthClientFilterKey =
  | "is_active"
  | "client_type"
  | "creator_type"
  | "broker"
  | "scope"
  | "created_at";

export type AdminOAuthClientFilterSelections =
  DataTableFilterSelections<AdminOAuthClientFilterKey>;
export type AdminOAuthClientFilterValueType = DataTableFilterValueType;
export type AdminOAuthClientFilterOperator = DataTableFilterOperator;
export type AdminOAuthClientFilterOption = DataTableFilterOption;
export type AdminOAuthClientFilterField =
  DataTableFilterField<AdminOAuthClientFilterKey>;

export type AdminOAuthClientSearchFieldKey =
  | "client"
  | "client_type"
  | "created_by"
  | "allowed_scopes";

export type AdminOAuthClientSearchField =
  DataTableSearchField<AdminOAuthClientSearchFieldKey>;
export type AdminOAuthClientSearchFilter =
  DataTableSearchGroup<AdminOAuthClientSearchFieldKey>;

export interface AdminOAuthClientFilterOptions {
  readonly client_types: readonly AdminOAuthClientTypeFilter[];
  readonly creator_types: readonly AdminOAuthClientCreatorType[];
  readonly broker_filters: readonly AdminOAuthClientBrokerFilter[];
  readonly statuses: readonly boolean[];
  readonly allowed_scopes: readonly string[];
  readonly sorts: readonly AdminOAuthClientSort[];
  /** Optional while older backend instances remain in a rolling deployment. */
  readonly fields?: readonly AdminOAuthClientFilterField[];
  /** Optional while older backend instances remain in a rolling deployment. */
  readonly search_fields?: readonly AdminOAuthClientSearchField[];
}

export interface AdminOAuthClientListResponse {
  readonly clients: readonly AdminOAuthClient[];
  readonly total: number;
  readonly page: number;
  readonly per_page: number;
  readonly filter_options: AdminOAuthClientFilterOptions;
}

export interface AdminOAuthClientListParams {
  readonly page: number;
  readonly per_page: 25 | 50 | 100;
  readonly search?: string;
  readonly search_filters?: string;
  readonly client_type?: string;
  readonly creator_type?: string;
  readonly broker?: string;
  readonly is_active?: boolean | string;
  readonly scope?: string;
  readonly created_dates?: string;
  readonly created_from?: string;
  readonly created_to?: string;
  readonly sort: AdminOAuthClientSort;
}

export interface AdminOAuthClientSearchState {
  readonly page?: number;
  readonly per_page?: 25 | 50 | 100;
  readonly search?: string;
  readonly search_filters?: string;
  readonly client_type?: string;
  readonly creator_type?: string;
  readonly broker?: string;
  readonly is_active?: boolean | string;
  readonly scope?: string;
  readonly created_dates?: string;
  readonly created_from?: string;
  readonly created_to?: string;
  readonly sort?: AdminOAuthClientSort;
}

export interface UpdateAdminOAuthClientRequest {
  readonly broker_capability_enabled?: boolean;
  readonly is_active?: boolean;
  readonly redirect_uris?: readonly string[];
  readonly allowed_scopes?: readonly string[];
  readonly client_name?: string;
}

export type BrokerPolicySource = "env_default" | "override";

export interface BrokerPolicyField {
  readonly effective: boolean;
  readonly env_default: boolean;
  readonly override: boolean | null;
  readonly source: BrokerPolicySource;
}

export interface BrokerSettingsResponse {
  readonly broker_require_sender_constraint: BrokerPolicyField;
  readonly broker_require_admin_capability: BrokerPolicyField;
}

export interface UpdateBrokerSettingsRequest {
  readonly broker_require_sender_constraint?: boolean | null;
  readonly broker_require_admin_capability?: boolean | null;
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

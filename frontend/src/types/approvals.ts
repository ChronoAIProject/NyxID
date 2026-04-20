export interface NotificationSettings {
  readonly telegram_connected: boolean;
  readonly telegram_username: string | null;
  readonly telegram_enabled: boolean;
  readonly push_enabled: boolean;
  readonly push_device_count: number;
  readonly approval_required: boolean;
  readonly approval_timeout_secs: number;
  readonly grant_expiry_days: number;
}

export interface PushDeviceItem {
  readonly device_id: string;
  readonly platform: "fcm" | "apns";
  readonly device_name: string | null;
  readonly registered_at: string;
  readonly last_used_at: string | null;
}

export interface PushDevicesResponse {
  readonly devices: readonly PushDeviceItem[];
  readonly push_enabled: boolean;
}

export interface RemoveDeviceResponse {
  readonly message: string;
}

export interface UpdateNotificationSettingsRequest {
  readonly telegram_enabled?: boolean;
  readonly push_enabled?: boolean;
  readonly approval_required?: boolean;
  readonly approval_timeout_secs?: number;
  readonly grant_expiry_days?: number;
}

export interface TelegramLinkResponse {
  readonly link_code: string;
  readonly bot_username: string;
  readonly expires_in_secs: number;
  readonly instructions: string;
}

export interface TelegramDisconnectResponse {
  readonly message: string;
}

export interface ApprovalRequestItem {
  readonly id: string;
  readonly service_name: string;
  readonly service_slug: string;
  readonly requester_type: string;
  readonly requester_label: string | null;
  readonly operation_summary: string;
  readonly action_description: string | null;
  /** Tool approval fields (null/undefined for proxy-initiated approvals) */
  readonly tool_name?: string | null;
  readonly tool_call_id?: string | null;
  readonly tool_arguments?: string | null;
  readonly is_destructive?: boolean | null;
  readonly approval_mode: ApprovalMode;
  readonly status: "pending" | "approved" | "rejected" | "expired";
  readonly created_at: string;
  readonly decided_at: string | null;
  readonly decision_channel: string | null;
}

export interface ApprovalRequestListResponse {
  readonly requests: readonly ApprovalRequestItem[];
  readonly total: number;
  readonly page: number;
  readonly per_page: number;
}

export interface ApprovalGrantItem {
  readonly id: string;
  readonly service_id: string;
  readonly service_name: string;
  readonly requester_type: string;
  readonly requester_id: string;
  readonly requester_label: string | null;
  readonly granted_at: string;
  readonly expires_at: string;
}

export interface ApprovalGrantListResponse {
  readonly grants: readonly ApprovalGrantItem[];
  readonly total: number;
  readonly page: number;
  readonly per_page: number;
}

export interface ApprovalDecideRequest {
  readonly approved: boolean;
}

export interface ApprovalDecideResponse {
  readonly id: string;
  readonly status: string;
  readonly decided_at: string;
}

export interface RevokeGrantResponse {
  readonly message: string;
}

export type ApprovalMode = "per_request" | "grant";

export interface ServiceApprovalConfigItem {
  readonly service_id: string;
  readonly service_name: string;
  readonly approval_required: boolean;
  readonly approval_mode: ApprovalMode;
  readonly created_at: string;
  readonly updated_at: string;
  /**
   * The owner's `UserService.id` that this policy applies to, when the
   * config can be traced back to an active user service. Absent for
   * legacy catalog-keyed configs whose backing UserService has been
   * deleted, and for policies set via the legacy DownstreamService path.
   * Clients should prefer this over `service_id` when cross-referencing
   * against `/user-services`.
   */
  readonly user_service_id?: string;
  /** Proxy slug of the matching `UserService`, for display. */
  readonly user_service_slug?: string;
}

export interface DominantOrgPolicy {
  readonly org_id: string;
  readonly service_id: string;
}

export interface ServiceApprovalConfigsResponse {
  readonly configs: readonly ServiceApprovalConfigItem[];
  /**
   * `(org_id, service_id)` pairs where an org the caller is a member of
   * has set its own per-service approval policy. Those org policies are
   * dominant in `resolve_org_aware_approval` **for that specific org**,
   * so the UI hides the matching entry from the personal Add-Override
   * picker — but only when the picker entry is inherited from that same
   * org. If the user inherits the same catalog service from a different
   * org without a dominant policy, the personal override still applies
   * and the entry stays selectable. Omitted by the server when empty —
   * treat as empty when absent. Only populated on the personal list
   * (no `?org_id`).
   */
  readonly dominant_org_policies?: readonly DominantOrgPolicy[];
}

export interface SetServiceApprovalConfigRequest {
  readonly approval_required?: boolean;
  readonly approval_mode?: ApprovalMode;
}

export interface SetServiceApprovalConfigResponse {
  readonly service_id: string;
  readonly service_name: string;
  readonly approval_required: boolean;
  readonly approval_mode: ApprovalMode;
  readonly created_at: string;
  readonly updated_at: string;
  readonly user_service_id?: string;
  readonly user_service_slug?: string;
}

export interface DeleteServiceApprovalConfigResponse {
  readonly message: string;
}

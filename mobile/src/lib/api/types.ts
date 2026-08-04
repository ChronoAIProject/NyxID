export type ChallengeStatus = "PENDING" | "APPROVED" | "DENIED" | "EXPIRED";
export type ApprovalMode = "per_request" | "grant";

export type ChallengeItem = {
  id: string;
  title: string;
  action: string;
  resource: string;
  approval_mode: ApprovalMode;
  risk_level: "low" | "medium" | "high";
  status: ChallengeStatus;
  created_at: string;
  expires_at: string;
  // Org context: populated when the backend reports the request was
  // created under an org's per-service approval policy. Optional so
  // the mobile build keeps working against backends that pre-date
  // the org-aware response fields.
  from_org_policy?: boolean;
  org_id?: string | null;
  org_name?: string | null;
};

export type ChallengeDetail = ChallengeItem & {
  summary: string;
  request_context: {
    client: string;
    location: string;
  };
  /**
   * Stable service identifier (e.g. catalog slug or `UserService.slug`).
   * Populated from `BackendApprovalRequestItem.service_slug`. Used for
   * telemetry (`mobile.approval_viewed`) so funnels group by the
   * underlying service rather than the user-editable display title.
   */
  service_slug: string;
};

export type ApprovalItem = {
  id: string;
  service_id: string;
  service_name: string;
  requester_type: string;
  requester_id: string;
  requester_label?: string | null;
  granted_at: string;
  expires_at: string;
  // Org context for grants. Mirrors the request fields above: both the
  // flag and the org id/name are optional so the app continues to
  // render personal-approval UI when the backend omits them.
  org_scoped?: boolean;
  org_id?: string | null;
  org_name?: string | null;
};

export type PageResponse<T> = {
  items: T[];
  page: number;
  per_page: number;
  total: number;
};

export type PushTokenRegisterRequest = {
  token: string;
  provider: "expo" | "apns" | "fcm";
  platform: "ios" | "android" | "web" | "unknown";
  previous_token?: string;
};

export type PushTokenRegisterResponse = {
  status: "REGISTERED" | "ROTATED";
  token: string;
  previous_token?: string;
};

export type NotificationSettings = {
  telegram_connected: boolean;
  telegram_username: string | null;
  telegram_enabled: boolean;
  approval_required: boolean;
  approval_timeout_secs: number;
  grant_expiry_days: number;
  push_enabled: boolean;
  push_device_count: number;
};

export type TelegramLinkInfo = {
  link_code: string;
  bot_username: string;
  expires_in_secs: number;
  instructions: string;
};

export type UpdateNotificationSettingsInput = {
  telegram_enabled?: boolean;
  push_enabled?: boolean;
  approval_required?: boolean;
  approval_timeout_secs?: number;
  grant_expiry_days?: number;
};

export type SubmitDecisionOptions = {
  idempotencyKey?: string;
};

export type AccountProfile = {
  id: string;
  email: string;
  display_name?: string | null;
  avatar_url?: string | null;
  email_verified: boolean;
  mfa_enabled: boolean;
  is_admin: boolean;
  is_active: boolean;
  social_provider?: string | null;
  created_at: string;
  last_login_at?: string | null;
};

export type DeleteAccountResponse = {
  status: "DELETED";
  deleted_at: string;
};

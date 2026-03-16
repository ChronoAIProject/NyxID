export type ChallengeStatus = "PENDING" | "APPROVED" | "DENIED" | "EXPIRED";

export type ChallengeItem = {
  id: string;
  title: string;
  action: string;
  resource: string;
  risk_level: "low" | "medium" | "high";
  status: ChallengeStatus;
  created_at: string;
  expires_at: string;
};

export type ChallengeDetail = ChallengeItem & {
  summary: string;
  request_context: {
    client: string;
    location: string;
  };
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
  grant_expiry_days: number;
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
  created_at: string;
  last_login_at?: string | null;
};

export type DeleteAccountResponse = {
  status: "DELETED";
  deleted_at: string;
};

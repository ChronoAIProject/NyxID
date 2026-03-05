export type ChallengeStatus = "PENDING" | "APPROVED" | "DENIED" | "EXPIRED";
export type ApprovalStatus = "ACTIVE" | "REVOKED" | "EXPIRED";

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
    ip: string;
    client: string;
    location: string;
  };
  allowed_durations_sec: number[];
  default_duration_sec: number;
};

export type ApprovalItem = {
  id: string;
  challenge_id: string;
  action: string;
  resource: string;
  status: ApprovalStatus;
  approved_at: string;
  expires_at: string;
  revoked_at?: string;
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

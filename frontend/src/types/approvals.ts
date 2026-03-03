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
  readonly telegram_enabled: boolean;
  readonly push_enabled: boolean;
  readonly approval_required: boolean;
  readonly approval_timeout_secs: number;
  readonly grant_expiry_days: number;
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

export interface ServiceApprovalConfigItem {
  readonly service_id: string;
  readonly service_name: string;
  readonly approval_required: boolean;
  readonly created_at: string;
  readonly updated_at: string;
}

export interface ServiceApprovalConfigsResponse {
  readonly configs: readonly ServiceApprovalConfigItem[];
}

export interface SetServiceApprovalConfigRequest {
  readonly approval_required: boolean;
}

export interface SetServiceApprovalConfigResponse {
  readonly service_id: string;
  readonly service_name: string;
  readonly approval_required: boolean;
  readonly created_at: string;
  readonly updated_at: string;
}

export interface DeleteServiceApprovalConfigResponse {
  readonly message: string;
}

export interface NodeMetadata {
  readonly agent_version: string | null;
  readonly os: string | null;
  readonly arch: string | null;
  readonly ip_address: string | null;
}

export interface NodeMetricsInfo {
  readonly total_requests: number;
  readonly success_count: number;
  readonly error_count: number;
  readonly success_rate: number;
  readonly avg_latency_ms: number;
  readonly last_error: string | null;
  readonly last_error_at: string | null;
  readonly last_success_at: string | null;
}

export interface NodeOwnerInfo {
  readonly kind: "user" | "org";
  readonly id: string;
  readonly display_name: string;
}

export interface NodeAdminInfo {
  readonly user_id: string;
  readonly display_name: string | null;
  readonly email: string | null;
  readonly role: "owner" | "admin";
}

export interface NodeInfo {
  readonly id: string;
  readonly name: string;
  readonly owner: NodeOwnerInfo;
  readonly status: string;
  readonly is_connected: boolean;
  readonly last_heartbeat_at: string | null;
  readonly connected_at: string | null;
  readonly metadata: NodeMetadata | null;
  readonly metrics: NodeMetricsInfo | null;
  readonly binding_count: number;
  readonly created_at: string;
}

export interface NodeListResponse {
  readonly nodes: readonly NodeInfo[];
}

export interface NodeAdminsResponse {
  readonly admins: readonly NodeAdminInfo[];
}

export interface CreateRegistrationTokenResponse {
  readonly token_id: string;
  readonly token: string;
  readonly name: string;
  readonly expires_at: string;
}

export interface RotateNodeTokenResponse {
  readonly auth_token: string;
  readonly signing_secret: string;
  readonly message: string;
}

export interface TransferNodeResponse {
  readonly node_id: string;
  readonly previous_owner: NodeOwnerInfo;
  readonly new_owner: NodeOwnerInfo;
  readonly deactivated_bindings_count: number;
  readonly cleared_user_service_count: number;
}

export type NodePendingCredentialInjectionMethod =
  | "header"
  | "query-param"
  | "path-prefix";

export interface NodePendingCredentialInfo {
  readonly id: string;
  readonly node_id: string;
  readonly service_slug: string;
  readonly injection_method: NodePendingCredentialInjectionMethod;
  readonly field_name: string;
  readonly target_url?: string;
  readonly label?: string;
  readonly created_by_user_id: string;
  readonly owner_user_id: string;
  readonly created_at: string;
  readonly expires_at: string;
  readonly consumed_at?: string;
  readonly declined_at?: string;
  readonly is_active: boolean;
}

export interface NodePendingCredentialsResponse {
  readonly pending_credentials: readonly NodePendingCredentialInfo[];
}

export interface AdminNodeInfo {
  readonly id: string;
  readonly name: string;
  readonly user_id: string;
  readonly user_email: string | null;
  readonly status: string;
  readonly is_connected: boolean;
  readonly last_heartbeat_at: string | null;
  readonly connected_at: string | null;
  readonly metadata: NodeMetadata | null;
  readonly metrics: NodeMetricsInfo | null;
  readonly binding_count: number;
  readonly created_at: string;
}

export interface AdminNodeListResponse {
  readonly nodes: readonly AdminNodeInfo[];
  readonly total: number;
  readonly page: number;
  readonly per_page: number;
}

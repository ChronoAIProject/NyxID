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

export interface NodeInfo {
  readonly id: string;
  readonly name: string;
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

export interface NodeBindingInfo {
  readonly id: string;
  readonly service_id: string;
  readonly service_name: string;
  readonly service_slug: string;
  readonly is_active: boolean;
  readonly priority: number;
  readonly created_at: string;
}

export interface BindingListResponse {
  readonly bindings: readonly NodeBindingInfo[];
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

export interface CreateBindingResponse {
  readonly id: string;
  readonly service_id: string;
  readonly service_name: string;
  readonly message: string;
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

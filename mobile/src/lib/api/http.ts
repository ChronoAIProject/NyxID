import {
  clearStoredAuthSession,
  loadStoredAuthSession,
  persistAuthSession,
} from "../auth/sessionStore";
import {
  AccountProfile,
  ApprovalItem,
  ChallengeDetail,
  ChallengeStatus,
  DeleteAccountResponse,
  PageResponse,
  PushTokenRegisterRequest,
  PushTokenRegisterResponse,
} from "./types";

const DEFAULT_API_BASE_URL = "http://localhost:3001/api/v1";
const ALWAYS_DURATION_SEC = 315360000;
const DEFAULT_CHALLENGE_DURATION_SEC = 24 * 60 * 60;
const CHALLENGE_PAGE_SIZE = 100;

type RequestOptions = {
  method?: "GET" | "POST" | "PATCH" | "PUT" | "DELETE";
  body?: unknown;
  requiresAuth?: boolean;
  headers?: Record<string, string>;
  retryOnAuthFailure?: boolean;
};

type LoginRequest = {
  email: string;
  password: string;
  mfa_code?: string;
};

type RegisterRequest = {
  email: string;
  password: string;
  display_name?: string;
};

type LoginResponse = {
  user_id: string;
  access_token: string;
  expires_in: number;
  refresh_token?: string;
};

type RegisterResponse = {
  user_id: string;
  message: string;
};

type RefreshResponse = {
  access_token: string;
  expires_in: number;
};

type SubmitDecisionResponse = {
  challenge_id: string;
  status: string;
  approval_id?: string;
};

type RevokeApprovalResponse = {
  approval_id: string;
  status: string;
};

type BackendApprovalRequestItem = {
  id: string;
  service_name: string;
  service_slug: string;
  requester_type: string;
  requester_label?: string | null;
  operation_summary: string;
  status: string;
  created_at: string;
};

type BackendApprovalRequestsResponse = {
  requests: BackendApprovalRequestItem[];
  total: number;
  page: number;
  per_page: number;
};

type BackendApprovalGrantItem = {
  id: string;
  service_name: string;
  requester_label?: string | null;
  granted_at: string;
  expires_at: string;
};

type BackendApprovalGrantsResponse = {
  grants: BackendApprovalGrantItem[];
  total: number;
  page: number;
  per_page: number;
};

type BackendDecideResponse = {
  id: string;
  status: string;
};

type BackendDeviceResponse = {
  device_id: string;
  platform: string;
  registered_at: string;
};

type MessageResponse = {
  message: string;
};

function getApiBaseUrl(): string {
  const rawBaseUrl = process.env.EXPO_PUBLIC_API_BASE_URL ?? DEFAULT_API_BASE_URL;
  const normalized = rawBaseUrl.replace(/\/+$/, "");

  if (normalized.endsWith("/mobile")) {
    return normalized.slice(0, -"/mobile".length);
  }

  return normalized;
}

function buildUrl(path: string): string {
  if (path.startsWith("http://") || path.startsWith("https://")) {
    return path;
  }

  const normalizedPath = path.startsWith("/") ? path : `/${path}`;
  return `${getApiBaseUrl()}${normalizedPath}`;
}

async function readJsonSafely(response: Response): Promise<unknown> {
  const text = await response.text();
  if (!text) return null;

  try {
    return JSON.parse(text) as unknown;
  } catch {
    return text;
  }
}

function stringifyErrorPayload(payload: unknown, status: number): string {
  if (payload && typeof payload === "object") {
    if ("error" in payload && typeof payload.error === "string") {
      return payload.error;
    }
    if ("message" in payload && typeof payload.message === "string") {
      return payload.message;
    }
  }

  if (typeof payload === "string" && payload.length > 0) {
    return payload;
  }

  return `request_failed_${status}`;
}

function parseOperationSummary(summary: string): { action: string; resource: string } {
  const normalized = summary.replace(/^proxy:/i, "").trim();
  const matched = normalized.match(/^([A-Z]+)\s+(.+)$/);
  if (matched) {
    return {
      action: matched[1] ?? "Request",
      resource: matched[2] ?? normalized,
    };
  }

  return {
    action: "Request",
    resource: normalized || "Unknown resource",
  };
}

function mapChallengeStatus(status: string): ChallengeStatus {
  if (status === "approved") return "APPROVED";
  if (status === "rejected") return "DENIED";
  if (status === "expired") return "EXPIRED";
  return "PENDING";
}

function deriveRiskLevel(action: string): "low" | "medium" | "high" {
  if (action === "DELETE" || action === "PUT" || action === "PATCH") return "high";
  if (action === "POST") return "medium";
  return "low";
}

function deriveChallengeExpiry(createdAt: string): string {
  const createdTime = Date.parse(createdAt);
  if (!Number.isFinite(createdTime)) {
    return new Date(Date.now() + DEFAULT_CHALLENGE_DURATION_SEC * 1000).toISOString();
  }
  return new Date(createdTime + DEFAULT_CHALLENGE_DURATION_SEC * 1000).toISOString();
}

function mapBackendRequestToChallenge(item: BackendApprovalRequestItem): ChallengeDetail {
  const parsed = parseOperationSummary(item.operation_summary);

  return {
    id: item.id,
    title: item.service_name,
    action: parsed.action,
    resource: parsed.resource,
    risk_level: deriveRiskLevel(parsed.action),
    status: mapChallengeStatus(item.status),
    created_at: item.created_at,
    expires_at: deriveChallengeExpiry(item.created_at),
    summary: item.operation_summary,
    request_context: {
      ip: "N/A",
      client: item.requester_type,
      location: item.requester_label ?? item.service_slug,
    },
    allowed_durations_sec: [DEFAULT_CHALLENGE_DURATION_SEC, ALWAYS_DURATION_SEC],
    default_duration_sec: DEFAULT_CHALLENGE_DURATION_SEC,
  };
}

function toBackendPushPlatform(platform: PushTokenRegisterRequest["platform"]): "apns" | "fcm" {
  if (platform === "ios") return "apns";
  if (platform === "android") return "fcm";
  throw new Error("push_platform_unsupported");
}

function resolveIosPushAppId(): string {
  const fromEnv = process.env.EXPO_PUBLIC_IOS_BUNDLE_ID?.trim();
  if (fromEnv) return fromEnv;
  return "fun.chrono-ai.nyxid";
}

function buildPushDevicePayload(payload: PushTokenRegisterRequest): {
  platform: "apns" | "fcm";
  token: string;
  app_id?: string;
  previous_token?: string;
} {
  const backendPlatform = toBackendPushPlatform(payload.platform);
  const previousToken =
    payload.previous_token && payload.previous_token !== payload.token
      ? payload.previous_token
      : undefined;
  if (backendPlatform === "apns") {
    return {
      platform: backendPlatform,
      token: payload.token,
      app_id: resolveIosPushAppId(),
      previous_token: previousToken,
    };
  }
  return {
    platform: backendPlatform,
    token: payload.token,
    previous_token: previousToken,
  };
}

async function requestRefreshAccessToken(): Promise<string | null> {
  const response = await fetch(buildUrl("/auth/refresh"), {
    method: "POST",
    headers: {
      Accept: "application/json",
    },
    credentials: "include",
  });

  const payload = await readJsonSafely(response);
  if (!response.ok) {
    return null;
  }

  if (payload && typeof payload === "object") {
    const maybeToken = (payload as RefreshResponse).access_token;
    if (typeof maybeToken === "string" && maybeToken.length > 0) {
      return maybeToken;
    }
  }

  return null;
}

async function requestJson<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const method = options.method ?? "GET";
  const requiresAuth = options.requiresAuth ?? true;
  const retryOnAuthFailure = options.retryOnAuthFailure ?? true;

  const headers: Record<string, string> = {
    Accept: "application/json",
    ...options.headers,
  };

  if (options.body !== undefined) {
    headers["Content-Type"] = "application/json";
  }

  const session = requiresAuth ? await loadStoredAuthSession() : null;

  if (requiresAuth) {
    if (!session?.accessToken) {
      throw new Error("auth_session_missing");
    }
    headers.Authorization = `Bearer ${session.accessToken}`;
  }

  const send = () =>
    fetch(buildUrl(path), {
      method,
      headers,
      body: options.body === undefined ? undefined : JSON.stringify(options.body),
      credentials: "include",
    });

  let response = await send();
  let payload = await readJsonSafely(response);

  if (response.status === 401 && requiresAuth && retryOnAuthFailure) {
    const refreshedAccessToken = await requestRefreshAccessToken();
    if (refreshedAccessToken) {
      await persistAuthSession({
        accessToken: refreshedAccessToken,
        refreshToken: session?.refreshToken,
      });
      headers.Authorization = `Bearer ${refreshedAccessToken}`;
      response = await send();
      payload = await readJsonSafely(response);
    }
  }

  if (!response.ok) {
    if (response.status === 401 && requiresAuth) {
      await clearStoredAuthSession();
    }
    throw new Error(stringifyErrorPayload(payload, response.status));
  }

  return payload as T;
}

async function listPendingApprovalRequests(
  page = 1,
  perPage = CHALLENGE_PAGE_SIZE
): Promise<BackendApprovalRequestsResponse> {
  return requestJson<BackendApprovalRequestsResponse>(
    `/approvals/requests?status=pending&page=${page}&per_page=${perPage}`
  );
}

export async function loginWithPasswordRequest(payload: LoginRequest): Promise<LoginResponse> {
  return requestJson<LoginResponse>("/auth/login", {
    method: "POST",
    body: payload,
    requiresAuth: false,
  });
}

export async function registerWithPasswordRequest(
  payload: RegisterRequest
): Promise<RegisterResponse> {
  return requestJson<RegisterResponse>("/auth/register", {
    method: "POST",
    body: payload,
    requiresAuth: false,
    retryOnAuthFailure: false,
  });
}

export async function listChallengesRequest(): Promise<PageResponse<ChallengeDetail>> {
  const response = await listPendingApprovalRequests(1, CHALLENGE_PAGE_SIZE);
  return {
    items: response.requests.map(mapBackendRequestToChallenge),
    total: response.total,
    page: response.page,
    per_page: response.per_page,
  };
}

export async function getChallengeRequest(challengeId: string): Promise<ChallengeDetail> {
  try {
    const item = await requestJson<BackendApprovalRequestItem>(
      `/approvals/requests/${encodeURIComponent(challengeId)}`
    );
    return mapBackendRequestToChallenge(item);
  } catch (error) {
    if (error instanceof Error && error.message === "not_found") {
      throw new Error("challenge_not_found");
    }
    throw error;
  }
}

export async function submitChallengeDecisionRequest(
  challengeId: string,
  decision: "APPROVE" | "DENY",
  durationSec: number | undefined,
  idempotencyKey: string
): Promise<SubmitDecisionResponse> {
  const response = await requestJson<BackendDecideResponse>(
    `/approvals/requests/${encodeURIComponent(challengeId)}/decide`,
    {
      method: "POST",
      headers: {
        "Idempotency-Key": idempotencyKey,
      },
      body: {
        approved: decision === "APPROVE",
        duration_sec: decision === "APPROVE" ? durationSec : undefined,
      },
    }
  );

  return {
    challenge_id: response.id,
    status: mapChallengeStatus(response.status),
  };
}

export async function listApprovalsRequest(): Promise<PageResponse<ApprovalItem>> {
  const response = await requestJson<BackendApprovalGrantsResponse>(
    "/approvals/grants?page=1&per_page=100"
  );

  return {
    items: response.grants.map((item) => ({
      id: item.id,
      challenge_id: item.id,
      action: "Approved Access",
      resource: item.requester_label
        ? `${item.service_name} · ${item.requester_label}`
        : item.service_name,
      status: "ACTIVE",
      approved_at: item.granted_at,
      expires_at: item.expires_at,
    })),
    total: response.total,
    page: response.page,
    per_page: response.per_page,
  };
}

export async function revokeApprovalRequest(approvalId: string): Promise<RevokeApprovalResponse> {
  await requestJson<{ message: string }>(`/approvals/grants/${encodeURIComponent(approvalId)}`, {
    method: "DELETE",
  });

  return {
    approval_id: approvalId,
    status: "REVOKED",
  };
}

export async function registerPushTokenRequest(
  payload: PushTokenRegisterRequest
): Promise<PushTokenRegisterResponse> {
  await requestJson<BackendDeviceResponse>("/notifications/devices", {
    method: "POST",
    body: buildPushDevicePayload(payload),
  });

  return {
    status: "REGISTERED",
    token: payload.token,
    previous_token: payload.previous_token,
  };
}

export async function rotatePushTokenRequest(
  payload: PushTokenRegisterRequest
): Promise<PushTokenRegisterResponse> {
  await requestJson<BackendDeviceResponse>("/notifications/devices", {
    method: "POST",
    body: buildPushDevicePayload(payload),
  });

  return {
    status: "ROTATED",
    token: payload.token,
    previous_token: payload.previous_token,
  };
}

export async function unregisterPushTokenRequest(
  payload: PushTokenRegisterRequest
): Promise<void> {
  await requestJson<MessageResponse>("/notifications/devices/current", {
    method: "DELETE",
    body: buildPushDevicePayload(payload),
  });
}

export async function getCurrentUserProfileRequest(): Promise<AccountProfile> {
  return requestJson<AccountProfile>("/users/me");
}

export async function deleteCurrentUserAccountRequest(): Promise<DeleteAccountResponse> {
  return requestJson<DeleteAccountResponse>("/users/me", {
    method: "DELETE",
  });
}

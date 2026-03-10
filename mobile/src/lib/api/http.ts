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
  NotificationSettings,
  PageResponse,
  PushTokenRegisterRequest,
  PushTokenRegisterResponse,
} from "./types";

const DEFAULT_API_BASE_URL = "http://localhost:3001/api/v1";
const DEFAULT_CHALLENGE_DURATION_SEC = 24 * 60 * 60;
const CHALLENGE_PAGE_SIZE = 100;
const REFRESH_CONFLICT_RETRY_COUNT = 1;
const REFRESH_CONFLICT_RETRY_DELAY_MS = 160;
const PROACTIVE_REFRESH_BUFFER_MS = 2 * 60 * 1000;

// ── Session invalidation hook ──────────────────────────────────────
// Allows AuthSessionContext to subscribe so that a 401 after failed
// refresh triggers a full sign-out (state + storage + push cleanup)
// rather than just clearing SecureStore behind React's back.
type SessionInvalidationListener = () => void;
let onSessionInvalidated: SessionInvalidationListener | null = null;

export function setSessionInvalidationListener(listener: SessionInvalidationListener | null) {
  onSessionInvalidated = listener;
}

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
  refresh_token: string;
};

type SubmitDecisionResponse = {
  challenge_id: string;
  status: string;
  approval_id?: string;
};

type RevokeApprovalResponse = {
  message: string;
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
  service_id: string;
  service_name: string;
  requester_type: string;
  requester_id: string;
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

type BackendNotificationSettingsResponse = {
  grant_expiry_days: number;
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

function computeAccessTokenExpiresAt(expiresInSec: number): number | undefined {
  if (!Number.isFinite(expiresInSec) || expiresInSec <= 0) return undefined;
  return Date.now() + Math.floor(expiresInSec * 1000);
}

function normalizeBase64Url(value: string): string {
  const normalized = value.replace(/-/g, "+").replace(/_/g, "/");
  const padding = normalized.length % 4;
  if (padding === 0) return normalized;
  return normalized + "=".repeat(4 - padding);
}

function decodeJwtExpiryMs(accessToken: string): number | undefined {
  const payloadSection = accessToken.split(".")[1];
  if (!payloadSection) return undefined;
  if (typeof globalThis.atob !== "function") return undefined;

  try {
    const decodedPayload = globalThis.atob(normalizeBase64Url(payloadSection));
    const parsed = JSON.parse(decodedPayload) as { exp?: unknown };
    if (typeof parsed.exp !== "number" || !Number.isFinite(parsed.exp)) return undefined;
    return parsed.exp * 1000;
  } catch {
    return undefined;
  }
}

function resolveAccessTokenExpiresAt(session: {
  accessToken: string;
  accessTokenExpiresAt?: number;
}): number | undefined {
  if (
    typeof session.accessTokenExpiresAt === "number" &&
    Number.isFinite(session.accessTokenExpiresAt)
  ) {
    return session.accessTokenExpiresAt;
  }
  return decodeJwtExpiryMs(session.accessToken);
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

function isIpv4Address(value: string): boolean {
  const parts = value.split(".");
  if (parts.length !== 4) return false;

  return parts.every((part) => {
    if (!/^\d{1,3}$/.test(part)) return false;
    const num = Number(part);
    return num >= 0 && num <= 255;
  });
}

function isIpv6Address(value: string): boolean {
  if (!value.includes(":")) return false;
  return /^[0-9A-Fa-f:]+$/.test(value);
}

function sanitizeDisplayValue(value: string | null | undefined, fallback: string): string {
  if (typeof value !== "string") return fallback;
  const trimmed = value.trim();
  if (!trimmed) return fallback;
  if (isIpv4Address(trimmed) || isIpv6Address(trimmed)) return "Hidden";
  return trimmed;
}

function sanitizeOptionalDisplayValue(value: string | null | undefined): string | null {
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  if (!trimmed) return null;
  if (isIpv4Address(trimmed) || isIpv6Address(trimmed)) return "Hidden";
  return trimmed;
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
    title: sanitizeDisplayValue(item.service_name, "Unknown Service"),
    action: parsed.action,
    resource: parsed.resource,
    risk_level: deriveRiskLevel(parsed.action),
    status: mapChallengeStatus(item.status),
    created_at: item.created_at,
    expires_at: deriveChallengeExpiry(item.created_at),
    summary: item.operation_summary,
    request_context: {
      client: sanitizeDisplayValue(item.requester_type, "Unknown"),
      location: sanitizeDisplayValue(
        item.requester_label ?? item.service_slug,
        "Unknown"
      ),
    },
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

let inFlightRefreshRequest: Promise<RefreshResponse | null> | null = null;

async function requestRefreshAccessToken(
  refreshToken: string,
  max409Retries = REFRESH_CONFLICT_RETRY_COUNT
): Promise<RefreshResponse | null> {
  for (let attempt = 0; attempt <= max409Retries; attempt += 1) {
    const response = await fetch(buildUrl("/auth/refresh"), {
      method: "POST",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/json",
      },
      body: JSON.stringify({ refresh_token: refreshToken, client: "mobile" }),
      credentials: "include",
    });

    const payload = await readJsonSafely(response);

    if (response.status === 409 && attempt < max409Retries) {
      await new Promise((resolve) => setTimeout(resolve, REFRESH_CONFLICT_RETRY_DELAY_MS));
      continue;
    }

    if (!response.ok) {
      if (__DEV__) {
        console.warn("[auth] refresh failed", response.status, payload);
      }
      return null;
    }

    if (payload && typeof payload === "object") {
      const data = payload as Partial<RefreshResponse>;
      if (typeof data.access_token === "string" && data.access_token.length > 0) {
        return {
          access_token: data.access_token,
          expires_in:
            typeof data.expires_in === "number" && Number.isFinite(data.expires_in)
              ? data.expires_in
              : 0,
          refresh_token:
            typeof data.refresh_token === "string" && data.refresh_token.length > 0
              ? data.refresh_token
              : refreshToken,
        };
      }
    }

    return null;
  }

  return null;
}

async function refreshAccessTokenWithSingleFlight(): Promise<RefreshResponse | null> {
  if (inFlightRefreshRequest) {
    return inFlightRefreshRequest;
  }

  inFlightRefreshRequest = (async () => {
    const session = await loadStoredAuthSession();
    if (!session?.refreshToken) {
      return null;
    }

    const refreshed = await requestRefreshAccessToken(session.refreshToken);
    if (!refreshed) {
      return null;
    }

    const nextRefreshToken = refreshed.refresh_token || session.refreshToken;
    await persistAuthSession({
      accessToken: refreshed.access_token,
      refreshToken: nextRefreshToken,
      accessTokenExpiresAt: computeAccessTokenExpiresAt(refreshed.expires_in),
    });

    return {
      ...refreshed,
      refresh_token: nextRefreshToken,
    };
  })().finally(() => {
    inFlightRefreshRequest = null;
  });

  return inFlightRefreshRequest;
}

export async function refreshAccessTokenIfNeeded(): Promise<boolean> {
  const session = await loadStoredAuthSession();
  if (!session?.accessToken || !session.refreshToken) {
    return false;
  }

  const expiresAt = resolveAccessTokenExpiresAt(session);
  if (typeof expiresAt !== "number" || !Number.isFinite(expiresAt)) {
    return false;
  }

  const remainingMs = expiresAt - Date.now();
  if (remainingMs > PROACTIVE_REFRESH_BUFFER_MS) {
    return false;
  }

  const refreshed = await refreshAccessTokenWithSingleFlight();
  return Boolean(refreshed);
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
    const refreshed = await refreshAccessTokenWithSingleFlight();
    if (refreshed) {
      headers.Authorization = `Bearer ${refreshed.access_token}`;
      response = await send();
      payload = await readJsonSafely(response);
    }
  }

  if (!response.ok) {
    if (response.status === 401 && requiresAuth) {
      if (onSessionInvalidated) {
        onSessionInvalidated();
      } else {
        await clearStoredAuthSession();
      }
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

export async function getNotificationSettingsRequest(): Promise<NotificationSettings> {
  const response = await requestJson<BackendNotificationSettingsResponse>(
    "/notifications/settings"
  );
  return {
    grant_expiry_days: response.grant_expiry_days,
  };
}

export async function loginWithPasswordRequest(payload: LoginRequest): Promise<LoginResponse> {
  return requestJson<LoginResponse>("/auth/login", {
    method: "POST",
    body: { ...payload, client: "mobile" },
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
      service_id: item.service_id,
      service_name: sanitizeDisplayValue(item.service_name, "Unknown Service"),
      requester_type: sanitizeDisplayValue(item.requester_type, "Unknown"),
      requester_id: sanitizeDisplayValue(item.requester_id, "unknown"),
      requester_label: sanitizeOptionalDisplayValue(item.requester_label),
      granted_at: item.granted_at,
      expires_at: item.expires_at,
    })),
    total: response.total,
    page: response.page,
    per_page: response.per_page,
  };
}

export async function revokeApprovalRequest(approvalId: string): Promise<RevokeApprovalResponse> {
  return requestJson<MessageResponse>(`/approvals/grants/${encodeURIComponent(approvalId)}`, {
    method: "DELETE",
  });
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

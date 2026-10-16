import Constants from "expo-constants";
import { fetch as expoFetch } from "expo/fetch";
import { executeAuthenticatedRequest, type RequestOptions } from "./authenticatedTransport";
import { mapBackendRequestToChallenge, mapChallengeStatus, type BackendApprovalRequestItem } from "./approvalAdapter";
import { getAuthEpoch } from "../auth/authEpoch";
import { readBoundedBytes } from "./readBoundedBytes";

import { ApiError } from "./ApiError";
import {
  clearStoredAuthSession,
  loadStoredAuthSession,
  persistRefreshedAuthSession,
} from "../auth/sessionStore";
import {
  ApprovalMode,
  AccountProfile,
  ApprovalItem,
  ChallengeDetail,
  DeleteAccountResponse,
  NotificationSettings,
  PageResponse,
  PushTokenRegisterRequest,
  PushTokenRegisterResponse,
  TelegramLinkInfo,
  UpdateNotificationSettingsInput,
} from "./types";

const DEFAULT_API_BASE_URL = "http://localhost:3001/api/v1";
const ACTIVITY_PAGE_SIZE = 20;
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
  // Org context for grants — optional for the same reason as above.
  org_scoped?: boolean;
  org_id?: string | null;
  org_name?: string | null;
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

type BackendNotificationSettingsResponse = NotificationSettings;

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

function buildApiError(payload: unknown, status: number): ApiError | Error {
  if (payload && typeof payload === "object") {
    const obj = payload as Record<string, unknown>;
    const errorKey = typeof obj.error === "string" ? obj.error : `request_failed_${status}`;
    const errorCode = typeof obj.error_code === "number" ? obj.error_code : 0;
    const message =
      typeof obj.message === "string" && obj.message.length > 0
        ? obj.message
        : errorKey;

    return new ApiError({ errorKey, errorCode, statusCode: status, message, details: obj.details });
  }

  if (typeof payload === "string" && payload.length > 0) {
    return new Error(payload);
  }

  return new Error(`request_failed_${status}`);
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

function toBackendPushPlatform(platform: PushTokenRegisterRequest["platform"]): "apns" | "fcm" {
  if (platform === "ios") return "apns";
  if (platform === "android") return "fcm";
  throw new Error("push_platform_unsupported");
}

function resolveIosPushAppId(): string {
  const fromConfig = Constants.expoConfig?.ios?.bundleIdentifier?.trim();
  if (fromConfig) return fromConfig;
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
    const epoch = getAuthEpoch();
    const session = await loadStoredAuthSession();
    if (!session?.refreshToken) {
      return null;
    }

    const refreshed = await requestRefreshAccessToken(session.refreshToken);
    if (!refreshed) {
      return null;
    }

    // An old refresh must never restore credentials after logout or replace
    // a newly signed-in account while its network request was in flight.
    const currentSession = await loadStoredAuthSession();
    if (!currentSession || currentSession.refreshToken !== session.refreshToken || epoch !== getAuthEpoch()) {
      return null;
    }

    const nextRefreshToken = refreshed.refresh_token || session.refreshToken;
    const persisted = await persistRefreshedAuthSession({
      accessToken: refreshed.access_token,
      refreshToken: nextRefreshToken,
      accessTokenExpiresAt: computeAccessTokenExpiresAt(refreshed.expires_in),
      userId: session.userId,
    }, epoch, session.refreshToken);
    if (!persisted) return null;

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

function clientHeaders(): Record<string, string> {
  return {
    "X-NyxID-Client": "mobile",
    "X-NyxID-Client-Version": Constants.expoConfig?.version ?? "dev",
  };
}

async function invalidateSession() {
  if (onSessionInvalidated) onSessionInvalidated();
  else await clearStoredAuthSession();
}

/** Expo's native fetch exposes a ReadableStream on iOS/Android; RN's global
 * fetch can buffer the entire response. Both paths share secure admission. */
export async function requestAuthenticatedResponse(path: string, options: RequestOptions = {}) {
  const { response } = await executeAuthenticatedRequest({
    url: buildUrl(path), options, clientHeaders: clientHeaders(),
    transport: expoFetch, loadSession: loadStoredAuthSession,
    refresh: refreshAccessTokenWithSingleFlight, invalidate: invalidateSession,
    toError: buildApiError, getAuthEpoch,
  });
  return response;
}

export async function requestJson<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const { response, assertOwner } = await executeAuthenticatedRequest({
    url: buildUrl(path), options, clientHeaders: clientHeaders(),
    transport: fetch, loadSession: loadStoredAuthSession,
    refresh: refreshAccessTokenWithSingleFlight, invalidate: invalidateSession,
    toError: buildApiError, getAuthEpoch,
  });
  const payload = await readJsonSafely(response);
  await assertOwner();
  return payload as T;
}

/** Raw upload shares the same secure admission and single 401 refresh as JSON. */
export async function requestBinaryJson<T>(
  path: string,
  bytes: Uint8Array,
  options: {
    headers?: Record<string, string>;
    signal?: AbortSignal;
    onProgress?: (percent: number) => void;
  } = {},
): Promise<T> {
  const { response, assertOwner } = await executeAuthenticatedRequest({
    url: buildUrl(path),
    options: {
      method: "POST",
      headers: options.headers,
      signal: options.signal,
      rawBody: new Uint8Array(bytes).buffer,
    },
    clientHeaders: clientHeaders(),
    transport: (url, init) =>
      new Promise<{ ok: boolean; status: number; text: () => Promise<string> }>(
        (resolve, reject) => {
          const xhr = new XMLHttpRequest();
          const abort = () => xhr.abort();
          const cleanup = () =>
            init.signal?.removeEventListener("abort", abort);
          xhr.open(init.method, url);
          xhr.timeout = 60_000;
          for (const [name, value] of Object.entries(init.headers))
            xhr.setRequestHeader(name, value);
          xhr.upload.onprogress = (event) => {
            if (event.lengthComputable)
              options.onProgress?.(
                Math.min(99, Math.round((event.loaded / event.total) * 100)),
              );
          };
          xhr.onerror = () => {
            cleanup();
            reject(
              new Error("Upload failed. Check your connection and try again."),
            );
          };
          xhr.ontimeout = () => {
            cleanup();
            reject(new Error("Upload timed out. Try a smaller file."));
          };
          xhr.onabort = () => {
            cleanup();
            reject(
              Object.assign(new Error("Upload cancelled."), {
                name: "AbortError",
              }),
            );
          };
          xhr.onload = () => {
            cleanup();
            resolve({
              ok: xhr.status >= 200 && xhr.status < 300,
              status: xhr.status,
              text: async () => xhr.responseText,
            });
          };
          if (init.signal?.aborted) {
            cleanup();
            reject(
              Object.assign(new Error("Upload cancelled."), {
                name: "AbortError",
              }),
            );
            return;
          }
          init.signal?.addEventListener("abort", abort, { once: true });
          xhr.send(init.body);
        },
      ),
    loadSession: loadStoredAuthSession,
    refresh: refreshAccessTokenWithSingleFlight,
    invalidate: invalidateSession,
    toError: buildApiError,
    getAuthEpoch,
  });
  const text = await response.text();
  await assertOwner();
  return JSON.parse(text) as T;
}

/** Bytes are consumed before the owner fence, so a late account response cannot be saved. */
export async function requestAuthenticatedBytes(
  path: string,
  signal?: AbortSignal,
): Promise<Uint8Array> {
  const { response, assertOwner } = await executeAuthenticatedRequest({
    url: buildUrl(path),
    options: { signal },
    clientHeaders: clientHeaders(),
    transport: expoFetch,
    loadSession: loadStoredAuthSession,
    refresh: refreshAccessTokenWithSingleFlight,
    invalidate: invalidateSession,
    toError: buildApiError,
    getAuthEpoch,
  });
  const bytes = await readBoundedBytes(response.body, 20 * 1024 * 1024);
  await assertOwner();
  return bytes;
}

async function listPendingApprovalRequests(
  page = 1,
  perPage = ACTIVITY_PAGE_SIZE
): Promise<BackendApprovalRequestsResponse> {
  // `include_admin_orgs=true` asks the backend to union in org-policy
  // requests for every org the caller is an active admin of. Backends
  // that don't yet recognize the param ignore it and return the legacy
  // personal-only list, so the mobile build remains compatible with
  // older deployments (see ChronoAIProject/NyxID#376).
  return requestJson<BackendApprovalRequestsResponse>(
    `/approvals/requests?status=pending&page=${page}&per_page=${perPage}&include_admin_orgs=true`
  );
}

export async function getNotificationSettingsRequest(): Promise<NotificationSettings> {
  return requestJson<NotificationSettings>("/notifications/settings");
}

export async function updateNotificationSettingsRequest(
  body: UpdateNotificationSettingsInput
): Promise<NotificationSettings> {
  return requestJson<NotificationSettings>("/notifications/settings", {
    method: "PUT",
    body,
  });
}

export async function telegramLinkRequest(): Promise<TelegramLinkInfo> {
  return requestJson<TelegramLinkInfo>("/notifications/telegram/link", {
    method: "POST",
  });
}

export async function telegramDisconnectRequest(): Promise<{ message: string }> {
  return requestJson<{ message: string }>("/notifications/telegram", {
    method: "DELETE",
  });
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

export async function listChallengesRequest(
  page = 1,
  perPage = ACTIVITY_PAGE_SIZE
): Promise<PageResponse<ChallengeDetail>> {
  const response = await listPendingApprovalRequests(page, perPage);
  return {
    items: response.requests.map(mapBackendRequestToChallenge),
    total: response.total,
    page: response.page,
    per_page: response.per_page,
  };
}

export async function listApprovalRequestsRequest(params?: {
  status?: string;
  page?: number;
  per_page?: number;
}): Promise<PageResponse<ChallengeDetail>> {
  const qs = new URLSearchParams();
  if (params?.status) qs.set("status", params.status);
  qs.set("page", String(params?.page ?? 1));
  qs.set("per_page", String(params?.per_page ?? 20));
  // See listPendingApprovalRequests for rationale. History queries use
  // the same opt-in so admins see their org's approved/rejected items
  // alongside their personal history.
  qs.set("include_admin_orgs", "true");

  const response = await requestJson<BackendApprovalRequestsResponse>(
    `/approvals/requests?${qs.toString()}`
  );
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
    if ((error instanceof ApiError && error.errorKey === "not_found") ||
        (error instanceof Error && error.message === "not_found")) {
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

export async function listApprovalsRequest(
  page = 1,
  perPage = ACTIVITY_PAGE_SIZE
): Promise<PageResponse<ApprovalItem>> {
  // Same admin-org opt-in as requests: mobile's Active tab shows
  // personal grants plus any org-scoped grants the caller admins.
  const response = await requestJson<BackendApprovalGrantsResponse>(
    `/approvals/grants?page=${page}&per_page=${perPage}&include_admin_orgs=true`
  );

  return {
    items: response.grants.map((item) => {
      const orgScoped = item.org_scoped === true;
      return {
        id: item.id,
        service_id: item.service_id,
        service_name: sanitizeDisplayValue(item.service_name, "Unknown Service"),
        requester_type: sanitizeDisplayValue(item.requester_type, "Unknown"),
        requester_id: sanitizeDisplayValue(item.requester_id, "unknown"),
        requester_label: sanitizeOptionalDisplayValue(item.requester_label),
        granted_at: item.granted_at,
        expires_at: item.expires_at,
        org_scoped: orgScoped,
        org_id: orgScoped ? item.org_id ?? null : undefined,
        org_name: orgScoped ? item.org_name ?? null : undefined,
      };
    }),
    total: response.total,
    page: response.page,
    per_page: response.per_page,
  };
}

export async function revokeApprovalRequest(
  approvalId: string,
  orgId?: string | null
): Promise<RevokeApprovalResponse> {
  // Org-scoped grants live under the owning org's user_id, not the
  // caller's. The backend's revoke handler uses `?org_id=` to pivot
  // ownership to the target org and then checks the caller is an
  // active admin of it. Without this param, DELETE on an org grant
  // 404s because the default path still searches by user_id = actor.
  const qs = orgId ? `?org_id=${encodeURIComponent(orgId)}` : "";
  return requestJson<MessageResponse>(
    `/approvals/grants/${encodeURIComponent(approvalId)}${qs}`,
    {
      method: "DELETE",
    }
  );
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

export async function getCurrentUserProfileRequest(signal?: AbortSignal): Promise<AccountProfile> {
  return requestJson<AccountProfile>("/users/me", { signal });
}

export async function deleteCurrentUserAccountRequest(): Promise<DeleteAccountResponse> {
  return requestJson<DeleteAccountResponse>("/users/me", {
    method: "DELETE",
  });
}

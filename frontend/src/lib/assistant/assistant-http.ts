import { ApiError } from "@/lib/api-client";
import { isTelemetryActive } from "@/lib/telemetry";
import { WireBodyCapture } from "@/lib/assistant/wire-body-capture";
import type { ApiErrorResponse } from "@/types/api";
import {
  captureAssistantWireLogHeader,
  captureAssistantWireLogId,
  useAssistantWireLogStore,
} from "@/stores/assistant-wire-log-store";
import { useAuthStore } from "@/stores/auth-store";

const API_PREFIX = "/api/v1";
const DEBUG_REQUEST_HEADER = "X-NyxID-Debug-Upstream";
const DEBUG_ID_HEADER = "X-NyxID-Debug-Upstream-Id";
const DEBUG_LOG_HEADER = "X-NyxID-Debug-Upstream-Log";
const MAX_CAPTURE_BYTES = 4 * 1024 * 1024;
const DEAD_SESSION_CODES = new Set([1001, 2000, 2001, 2002]);

type AssistantMethod = "GET" | "POST" | "DELETE";

export interface AssistantHttpRequest {
  readonly body?: unknown;
  readonly headers?: Readonly<Record<string, string>>;
  readonly method?: AssistantMethod;
  readonly signal?: AbortSignal;
}

export interface AssistantHttpMockRequest {
  readonly endpoint: string;
  readonly init: RequestInit;
}

export type AssistantHttpMockHandler = (
  request: AssistantHttpMockRequest,
) => Response | Promise<Response | undefined> | undefined;

declare global {
  // Installed by the dev/e2e assistant fixture world. Production never sets it.
  var __nyxidAssistantHttpMock: AssistantHttpMockHandler | undefined;
}

function isMockMode(): boolean {
  if (import.meta.env.MODE === "test") return true;
  if (!import.meta.env.DEV || typeof window === "undefined") return false;
  return new URLSearchParams(window.location.search).get("mock") === "1";
}

function conversationIdFromEndpoint(endpoint: string): string | null {
  const match = /^\/assistant\/conversations\/([^/?]+)/.exec(endpoint);
  if (!match?.[1]) return null;
  try {
    return decodeURIComponent(match[1]);
  } catch {
    return match[1];
  }
}

function suppressWireLog(endpoint: string): boolean {
  return (
    endpoint === "/assistant/conversations" ||
    endpoint.startsWith("/assistant/wire-logs/")
  );
}

function wireHeaders(endpoint: string): Record<string, string> {
  if (suppressWireLog(endpoint)) return {};
  const { featureEnabled, captureEnabled } =
    useAssistantWireLogStore.getState();
  return featureEnabled && captureEnabled
    ? { [DEBUG_REQUEST_HEADER]: "1" }
    : {};
}

async function captureResponse(
  endpoint: string,
  method: AssistantMethod,
  response: Response,
): Promise<void> {
  if (suppressWireLog(endpoint)) return;
  const store = useAssistantWireLogStore.getState();
  if (!store.featureEnabled || !store.captureEnabled) return;
  const meta = {
    kind: "header" as const,
    conversationId: conversationIdFromEndpoint(endpoint),
    label: `${method} ${endpoint}`,
    status: response.status,
  };
  const wireLogId = response.headers.get(DEBUG_ID_HEADER);
  const exchangeId = wireLogId
    ? captureAssistantWireLogId(wireLogId, meta)
    : captureAssistantWireLogHeader(response.headers.get(DEBUG_LOG_HEADER), meta);
  if (!exchangeId) return;

  const clone = response.clone();
  if (!clone.body) {
    store.attachResponseBody(exchangeId, "", 0, false);
    store.finalizeCapture(exchangeId, "complete");
    return;
  }

  const reader = clone.body.getReader();
  const capture = new WireBodyCapture(MAX_CAPTURE_BYTES);
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      capture.push(value);
      if (capture.truncated) {
        await reader.cancel().catch(() => undefined);
        break;
      }
    }
    const result = capture.finish();
    const latest = useAssistantWireLogStore.getState();
    latest.attachResponseBody(
      exchangeId,
      result.text,
      result.bytes,
      result.truncated,
    );
    latest.finalizeCapture(exchangeId, "complete");
  } catch {
    useAssistantWireLogStore
      .getState()
      .finalizeCapture(exchangeId, "network_error");
  } finally {
    reader.releaseLock();
  }
}

function fallbackError(status: number): ApiErrorResponse {
  return {
    error: "unknown_error",
    error_code: -1,
    message: `Request failed with status ${String(status)}`,
  };
}

async function parseError(response: Response): Promise<ApiErrorResponse> {
  try {
    const value = (await response.json()) as Record<string, unknown>;
    return {
      error: typeof value.error === "string" ? value.error : "unknown_error",
      error_code:
        typeof value.error_code === "number" ? value.error_code : -1,
      message:
        typeof value.message === "string" && value.message
          ? value.message
          : `Request failed with status ${String(response.status)}`,
      ...(typeof value.consent_url === "string"
        ? { consent_url: value.consent_url }
        : {}),
    };
  } catch {
    return fallbackError(response.status);
  }
}

function redirectToConsent(error: ApiErrorResponse): void {
  if (
    error.error !== "consent_required" ||
    !error.consent_url ||
    typeof window === "undefined"
  ) {
    return;
  }
  const url = error.consent_url;
  void import("@/lib/navigation").then(({ openExternal }) => openExternal(url));
}

async function mockResponse(
  endpoint: string,
  init: RequestInit,
): Promise<Response | undefined> {
  if (!isMockMode()) return undefined;
  return globalThis.__nyxidAssistantHttpMock?.({ endpoint, init });
}

export async function assistantHttp(
  endpoint: string,
  options: AssistantHttpRequest = {},
): Promise<Response> {
  const method = options.method ?? "GET";
  const init: RequestInit = {
    method,
    credentials: "include",
    headers: {
      "Content-Type": "application/json",
      ...(isTelemetryActive() ? { "X-NyxID-Client": "ui" } : {}),
      ...wireHeaders(endpoint),
      ...options.headers,
    },
    signal: options.signal,
    ...(options.body === undefined
      ? {}
      : { body: JSON.stringify(options.body) }),
  };
  const response =
    (await mockResponse(endpoint, init)) ??
    (await fetch(`${API_PREFIX}${endpoint}`, init));

  void captureResponse(endpoint, method, response).catch(() => undefined);
  if (response.ok) return response;

  const errorBody = await parseError(response);
  if (
    response.status === 401 &&
    DEAD_SESSION_CODES.has(errorBody.error_code)
  ) {
    useAuthStore.getState().setUser(null);
  }
  redirectToConsent(errorBody);
  throw new ApiError(response.status, errorBody);
}

export async function assistantJson<T>(
  endpoint: string,
  options: AssistantHttpRequest = {},
): Promise<T> {
  const response = await assistantHttp(endpoint, options);
  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

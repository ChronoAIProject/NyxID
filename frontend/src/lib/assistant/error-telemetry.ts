import { capture } from "@/lib/telemetry";
import type {
  NyxbotErrorCode,
  NyxbotErrorProperties,
} from "@/lib/telemetry-schema";

export interface ErrorCorrelation {
  readonly requestId?: string;
  readonly correlationId?: string;
  readonly traceId?: string;
}

interface NyxbotErrorContext extends ErrorCorrelation {
  readonly code: NyxbotErrorCode;
  readonly engine: NyxbotErrorProperties["engine"];
  readonly stage: NyxbotErrorProperties["stage"];
  readonly endpoint: string;
  readonly status?: number;
  readonly upstreamCode?: unknown;
  readonly conversationId?: string;
  readonly turnId?: string;
  readonly durationMs?: number;
}

const reportedErrors = new WeakSet<object>();

function errorChain(error: unknown): object[] {
  const chain: object[] = [];
  while (
    error !== null &&
    typeof error === "object" &&
    chain.length < 8 &&
    !chain.includes(error)
  ) {
    chain.push(error);
    error = "cause" in error ? error.cause : undefined;
  }
  return chain;
}

export function isNyxbotCancellation(error: unknown): boolean {
  return (
    error instanceof Error &&
    [
      "AbortError",
      "ReaderStoppedError",
      "AssistantTurnCancelledError",
    ].includes(error.name)
  );
}

// Allow only bounded identifiers, never messages, URLs, credentials, or bodies.
function safeIdentifier(value: unknown): string | undefined {
  if (typeof value !== "string" || !/^[\w.-]{1,128}$/.test(value)) return;
  if (/^(?:nyx_|nyxid_|sk-|gh[pousr]_|ph[cxs]_)/i.test(value)) return;
  return value;
}

function safeUpstreamCode(value: unknown): string | number | undefined {
  if (typeof value === "number" && Number.isSafeInteger(value) && value >= 0)
    return value;
  const code = safeIdentifier(value);
  return code && code.length <= 64 ? code : undefined;
}

function routePattern(endpoint: string): string {
  const path = endpoint.split("?")[0]?.replace(/^\/api\/v1/, "") ?? "";
  if (path === "/assistant") return path;
  if (path.startsWith("/assistant/nyxagent/")) {
    return path === "/assistant/nyxagent/turns"
      ? path
      : "/assistant/nyxagent/:resource";
  }
  if (path === "/assistant/chat") return path;
  if (/^\/assistant\/direct\/(completions|models|skills|efforts)$/.test(path)) {
    return path;
  }
  if (path === "/assistant/conversations") return path;
  if (/^\/assistant\/conversations\/[^/]+\/state$/.test(path)) {
    return "/assistant/conversations/:id/state";
  }
  if (/^\/assistant\/conversations\/[^/]+$/.test(path)) {
    return "/assistant/conversations/:id";
  }
  if (/^\/assistant\/wire-logs\//.test(path)) return "/assistant/wire-logs/:id";
  return "/assistant/:resource";
}

export function httpErrorCode(status: number): NyxbotErrorCode {
  if (status === 0) return "NYXBOT_NETWORK_ERROR";
  if (status === 401) return "NYXBOT_HTTP_UNAUTHORIZED";
  if (status === 403) return "NYXBOT_HTTP_FORBIDDEN";
  if (status === 429) return "NYXBOT_HTTP_RATE_LIMITED";
  if (status >= 500) return "NYXBOT_HTTP_SERVER_ERROR";
  return "NYXBOT_HTTP_ERROR";
}

export function responseCorrelation(response: Pick<Response, "headers">): ErrorCorrelation {
  return {
    requestId: response.headers.get("x-request-id") ?? undefined,
    correlationId: response.headers.get("x-correlation-id") ?? undefined,
    traceId:
      response.headers.get("X-NyxID-Trace-Id") ??
      response.headers.get("x-trace-id") ??
      undefined,
  };
}

/** Extract just the UUID from the displayed trace reference, never its message. */
export function messageTraceId(message: string): string | undefined {
  return /\btrace\s*id\s*:\s*([a-f\d]{8}-(?:[a-f\d]{4}-){3}[a-f\d]{12})\b/i.exec(
    message,
  )?.[1];
}

/** Error-only event through the existing consent-gated telemetry wrapper. */
export function captureNyxbotError(
  context: NyxbotErrorContext,
  error?: unknown,
): void {
  try {
    if (isNyxbotCancellation(error)) return;
    const chain = errorChain(error);
    if (chain.some((item) => reportedErrors.has(item))) return;
    for (const item of chain) reportedErrors.add(item);
    const props: NyxbotErrorProperties = {
      error_code: context.code,
      engine: context.engine,
      stage: context.stage,
      route: routePattern(context.endpoint),
      http_status: context.status,
      upstream_error_code: safeUpstreamCode(context.upstreamCode),
      request_id: safeIdentifier(context.requestId),
      correlation_id: safeIdentifier(context.correlationId),
      trace_id: safeIdentifier(context.traceId),
      conversation_id: safeIdentifier(context.conversationId),
      turn_id: safeIdentifier(context.turnId),
      duration_ms:
        context.durationMs === undefined || !Number.isFinite(context.durationMs)
          ? undefined
          : Math.min(86_400_000, Math.max(0, Math.round(context.durationMs))),
    };
    void Promise.resolve(capture({ name: "nyxbot.error", props })).catch(
      () => undefined,
    );
  } catch {
    // Diagnostic failures must never change chat settlement or error handling.
  }
}

/** Last resort for an unexpected error; deliberately excludes its text and stack. */
export function captureUnexpectedNyxbotError(
  error: unknown,
  endpoint = "/assistant",
): void {
  captureNyxbotError(
    {
      code: "NYXBOT_UNKNOWN_ERROR",
      engine: "unknown",
      stage: "runtime",
      endpoint,
    },
    error,
  );
}

/** Capture only the active Nyxbot surface, including non-bubbling asset errors. */
export function installNyxbotErrorFallback(
  target: Window = window,
): () => void {
  const active = () => /^\/assistant(?:\/|$)/.test(target.location.pathname);
  const onError = (event: Event) => {
    if (!active()) return;
    if (event instanceof ErrorEvent) {
      captureUnexpectedNyxbotError(
        event.error ?? event,
        target.location.pathname,
      );
    } else if (
      event.target instanceof Element &&
      ["IMG", "SCRIPT", "LINK"].includes(event.target.tagName)
    ) {
      captureNyxbotError(
        {
          code: "NYXBOT_RESOURCE_ERROR",
          engine: "unknown",
          stage: "resource",
          endpoint: target.location.pathname,
        },
        event.target,
      );
    }
  };
  const onRejection = (event: PromiseRejectionEvent) => {
    if (active())
      captureUnexpectedNyxbotError(event.reason, target.location.pathname);
  };
  target.addEventListener("error", onError, true);
  target.addEventListener("unhandledrejection", onRejection);
  return () => {
    target.removeEventListener("error", onError, true);
    target.removeEventListener("unhandledrejection", onRejection);
  };
}

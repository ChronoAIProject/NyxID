import { translateApp } from "../../i18n/appTranslation";

/**
 * Structured API error that preserves all fields from the backend ErrorResponse.
 *
 * Backend shape: { error: string, error_code: number, message: string, ...extras }
 */
export class ApiError extends Error {
  /** Machine-readable error key, e.g. "bad_request", "not_found" */
  readonly errorKey: string;
  /** Canonical source for locally generated errors only; server text is raw. */
  readonly sourceMessage?: string;
  /** Numeric error code for client-side mapping (1000-9002) */
  readonly errorCode: number;
  /** HTTP status code */
  readonly statusCode: number;
  /** True only for a NyxID JSON error with a numeric code, not a proxy fallback. */
  readonly hasErrorEnvelope: boolean;
  /** Optional structured validation diagnostics; rendered only by their owner. */
  readonly details?: unknown;
  readonly reason?: string;
  readonly requestId?: string;
  readonly traceId?: string;
  readonly retryAfterMs?: number;

  constructor(opts: {
    errorKey: string;
    errorCode: number;
    statusCode: number;
    hasErrorEnvelope?: boolean;
    message: string;
    sourceMessage?: string;
    details?: unknown;
    reason?: string;
    requestId?: string;
    traceId?: string;
    retryAfterMs?: number;
  }) {
    super(opts.message);
    this.name = "ApiError";
    this.sourceMessage = opts.sourceMessage;
    if (opts.sourceMessage) {
      const source = opts.sourceMessage;
      // Lazy stack formatters must not call the display getter during capture.
      void this.stack;
      // Direct message consumers also format when they display the retained
      // error. Never freeze a local fallback into its creation-time locale.
      Object.defineProperty(this, "message", { configurable: true, get: () => translateApp(source) });
    }
    this.errorKey = opts.errorKey;
    this.errorCode = opts.errorCode;
    this.statusCode = opts.statusCode;
    this.hasErrorEnvelope = opts.hasErrorEnvelope ?? Number.isInteger(opts.errorCode);
    this.details = opts.details;
    this.reason = opts.reason;
    this.requestId = opts.requestId;
    this.traceId = opts.traceId;
    this.retryAfterMs = opts.retryAfterMs;
  }
}

/** Type guard for ApiError */
export function isApiError(error: unknown): error is ApiError {
  return error instanceof ApiError;
}

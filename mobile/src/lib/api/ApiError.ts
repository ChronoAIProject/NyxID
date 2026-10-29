/**
 * Structured API error that preserves all fields from the backend ErrorResponse.
 *
 * Backend shape: { error: string, error_code: number, message: string, ...extras }
 */
export class ApiError extends Error {
  /** Machine-readable error key, e.g. "bad_request", "not_found" */
  readonly errorKey: string;
  /** Numeric error code for client-side mapping (1000-9002) */
  readonly errorCode: number;
  /** HTTP status code */
  readonly statusCode: number;
  /** Optional structured validation diagnostics; rendered only by their owner. */
  readonly details?: unknown;
  readonly reason?: string;

  constructor(opts: {
    errorKey: string;
    errorCode: number;
    statusCode: number;
    message: string;
    details?: unknown;
    reason?: string;
  }) {
    super(opts.message);
    this.name = "ApiError";
    this.errorKey = opts.errorKey;
    this.errorCode = opts.errorCode;
    this.statusCode = opts.statusCode;
    this.details = opts.details;
    this.reason = opts.reason;
  }
}

/** Type guard for ApiError */
export function isApiError(error: unknown): error is ApiError {
  return error instanceof ApiError;
}

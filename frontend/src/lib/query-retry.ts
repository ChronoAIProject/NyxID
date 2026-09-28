import { ApiError } from "@/lib/api-client";
import { isInsufficientCreditsHttp } from "@/lib/credits-denial";

/**
 * Default query retry: never retry a rejected session or a request the
 * billing gate refused for lack of credits; retrying cannot change either.
 */
export function shouldRetryQuery(
  failureCount: number,
  error: unknown,
): boolean {
  if (
    error &&
    typeof error === "object" &&
    "status" in error &&
    (error as { status: number }).status === 401
  ) {
    return false;
  }
  if (
    error instanceof ApiError &&
    isInsufficientCreditsHttp(error.status, error.errorResponse)
  ) {
    return false;
  }
  return failureCount < 3;
}

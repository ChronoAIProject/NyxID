import { ApiError } from "./ApiError";

/** Only the idempotent roster GET opts in. No transport/mutation replay. */
export function retryAgentRead(failureCount: number, error: unknown): boolean {
  return (
    failureCount < 2 &&
    error instanceof ApiError &&
    [0, 502, 503, 504].includes(error.statusCode)
  );
}
export const agentReadRetryDelay = (attempt: number) =>
  Math.min(1000 * 2 ** attempt, 2000);

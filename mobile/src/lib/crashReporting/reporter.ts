import { ApiError } from "../api/ApiError";
const ownErrors = new WeakSet<object>();
export function crashAuthorityError(): Error {
  const error = new Error("crash_authority_changed");
  ownErrors.add(error);
  return error;
}
export function excludedMobileError(error: unknown): boolean {
  return (error instanceof ApiError && error.statusCode < 500) ||
    (typeof error === "object" && error !== null && ownErrors.has(error));
}
import type { RelayRequest, AnalyticsPermission } from "../analyticsRelay";
import type { CrashReport } from "../../../../shared/crash-report";
import type { CrashBinding } from "./nativeRecord";
export function createCrashRelay(
  request: RelayRequest,
  isCurrent: (binding: CrashBinding) => boolean,
  denied: () => void,
  authorityVersion: () => number = () => 0,
) {
  let inFlight = 0;
  return async (
    report: CrashReport,
    binding: CrashBinding,
  ): Promise<boolean> => {
    if (inFlight >= 4 || !isCurrent(binding)) return false;
    const version = authorityVersion();
    const current = () => version === authorityVersion() && isCurrent(binding);
    const guard = () => {
      if (!current()) throw crashAuthorityError();
    };
    inFlight++;
    try {
      const options = {
        expectedAccountId: binding.accountId,
        authorityGuard: guard,
      };
      guard();
      const permission = await request<AnalyticsPermission>(
        "/users/me/preferences/analytics",
        options,
      );
      guard();
      if (
        !permission.enabled ||
        permission.policy_version !== "2" ||
        !permission.diagnostics_available ||
        permission.revision !== binding.revision ||
        permission.account_id !== binding.accountId
      ) {
        denied();
        return false;
      }
      await request("/users/me/crash-reports", {
        ...options,
        method: "POST",
        body: {
          account_id: binding.accountId,
          revision: binding.revision,
          report,
        },
      });
      guard();
      return true;
    } catch (error) {
      const code =
        error && typeof error === "object"
          ? (error as { errorCode?: number }).errorCode
          : undefined;
      if (current() && (code === 13400 || code === 13401)) denied();
      return false;
    } finally {
      inFlight--;
    }
  };
}

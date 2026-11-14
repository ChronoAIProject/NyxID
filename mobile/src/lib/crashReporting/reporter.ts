import { ApiError } from "../api/ApiError";
const ownErrors = new WeakSet<object>();
export function crashAuthorityError(): Error {
  const error = new Error("crash_authority_changed");
  ownErrors.add(error);
  return error;
}
export function excludedMobileError(error: unknown): boolean {
  return (
    (error instanceof ApiError && error.statusCode < 500) ||
    (typeof error === "object" && error !== null && ownErrors.has(error))
  );
}
import type { RelayRequest, AnalyticsPermission } from "../analyticsRelay";
import type { CrashReport } from "../../../../shared/crash-report";
import type { CrashBinding } from "./nativeRecord";
/** Missing status is an older/rolled-back server, never proof of withdrawal. */
export function permissionDecision(
  permission: AnalyticsPermission,
  accountId: string,
): "available" | "pause" | "disarm" {
  if (
    permission.account_id !== accountId ||
    !permission.enabled ||
    permission.diagnostics_status === "not_offered" ||
    (permission.diagnostics_status === "consent_required" &&
      permission.policy_version !== "2")
  )
    return "disarm";
  return permission.diagnostics_status === "available" &&
    permission.policy_version === "2" &&
    permission.diagnostics_available === true
    ? "available"
    : "pause";
}
export function createCrashRelay(
  request: RelayRequest,
  isCurrent: (binding: CrashBinding) => boolean,
  denied: () => void,
  authorityVersion: () => number = () => 0,
  paused: () => void = () => {},
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
      const decision = permissionDecision(permission, binding.accountId);
      if (decision !== "available") {
        if (decision === "disarm") denied();
        else paused();
        return false;
      }
      if (permission.revision !== binding.revision) {
        paused();
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
      if (!current()) return false;
      const authoritative = error instanceof ApiError && error.hasErrorEnvelope;
      if (authoritative && error.errorCode === 13400) {
        denied();
        return false;
      }
      if (authoritative && error.errorCode === 13401) {
        paused();
        return false;
      }
      if (
        authoritative &&
        [400, 404, 409, 410, 413, 415, 422].includes(error.statusCode)
      )
        return true; // A report-specific refusal drops this record only.
      paused();
      return false;
    } finally {
      inFlight--;
    }
  };
}

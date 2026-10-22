import { ApiError } from "../../lib/api/ApiError";
import type { AccountProfile } from "../../lib/api/types";

export function isMfaStepUpRequired(error: unknown): error is ApiError {
  return (
    error instanceof ApiError &&
    error.statusCode === 403 &&
    error.errorCode === 2003 &&
    error.errorKey === "mfa_step_up_required"
  );
}
export function mobileMfaEnabled(profile: AccountProfile): boolean {
  return (
    profile.capabilities?.enabled_features?.includes("auth:mobile-mfa") ?? false
  );
}

export async function shouldOfferStepUp(
  error: unknown,
  loadProfile: () => Promise<AccountProfile>,
  isCurrent: () => boolean,
): Promise<boolean> {
  if (!isMfaStepUpRequired(error) || !isCurrent()) return false;
  try {
    const profile = await loadProfile();
    return isCurrent() && mobileMfaEnabled(profile);
  } catch {
    return false;
  }
}

export async function completeStepUp(
  code: string,
  verify: (code: string) => Promise<void>,
  retry: () => Promise<void>,
  isCurrent: () => boolean,
): Promise<void> {
  if (!isCurrent()) return;
  await verify(code);
  if (isCurrent()) await retry();
}

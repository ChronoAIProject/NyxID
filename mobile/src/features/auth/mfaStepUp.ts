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

export type StepUpScope = { scope: string; generation: number };

export function stepUpScopeIsCurrent(
  origin: StepUpScope,
  activeScope: string | null,
  activeGeneration: number,
): boolean {
  return origin.scope === activeScope && origin.generation === activeGeneration;
}

export async function decideStepUp(
  error: unknown,
  loadProfile: () => Promise<AccountProfile>,
  isCurrent: () => boolean,
): Promise<"prompt" | "message" | "discard" | "unhandled"> {
  if (!isCurrent()) return "discard";
  if (!isMfaStepUpRequired(error)) return "unhandled";
  try {
    const profile = await loadProfile();
    if (!isCurrent()) return "discard";
    return mobileMfaEnabled(profile) ? "prompt" : "message";
  } catch {
    return isCurrent() ? "message" : "discard";
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

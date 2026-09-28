import { getAssistantIdentityUserId } from "@/lib/assistant/identity";
import { isPublicPath } from "@/lib/public-paths";
import { useCreditsDenialStore } from "@/stores/credits-denial-store";

export const INSUFFICIENT_CREDITS = "insufficient_credits";
/**
 * Numeric code of `AppError::InsufficientCredits`. `ConnectLinkNotFound`
 * (HTTP 404) shares it, so it only counts on a 402 without an `error` symbol.
 */
const INSUFFICIENT_CREDITS_CODE = 11300;

/** Whose credits a denied operation would have spent. */
export type CreditsPayer =
  | "self"
  | "unknown"
  | { readonly org: { readonly id: string; readonly name?: string } };

/**
 * Opt-in marker for a foreground request. Background reads, polling and
 * pagination never set it, so they can never interrupt the user.
 */
export interface CreditsDenialRequest {
  /** Operation identity: a dismissed key never prompts again. */
  readonly key: string;
  readonly payer: CreditsPayer;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

/** A NyxID error envelope for HTTP 402 `insufficient_credits`. */
export function isInsufficientCreditsHttp(
  status: number,
  body: unknown,
): boolean {
  if (status !== 402 || !isRecord(body)) return false;
  if (body.error !== undefined && body.error !== null) {
    return body.error === INSUFFICIENT_CREDITS;
  }
  return body.error_code === INSUFFICIENT_CREDITS_CODE;
}

/** A stream or turn error code; exact symbol only, never numeric. */
export function isInsufficientCreditsCode(code: unknown): boolean {
  return code === INSUFFICIENT_CREDITS;
}

/** The signed-in identity to capture when a foreground operation starts. */
export function currentCreditsActor(): string | null {
  return getAssistantIdentityUserId();
}

export function notifyCreditsDenied(
  request: CreditsDenialRequest,
  actorId: string | null,
): void {
  useCreditsDenialStore.getState().notify({ ...request, actorId });
}

/** Notify for an opted-in HTTP failure; callers still throw their error. */
export function reportCreditsDenialHttp(
  request: CreditsDenialRequest | undefined,
  status: number,
  body: unknown,
  actorId: string | null,
): void {
  if (request && isInsufficientCreditsHttp(status, body)) {
    notifyCreditsDenied(request, actorId);
  }
}

/**
 * Payer for a resource owned by `ownerId` (a person or org user id, as on
 * channel bots). Anything other than the caller is an org wallet. Owner-billed
 * operations must know their owner: an unknown payer would offer a personal
 * purchase for an org wallet, so callers without one do not opt in.
 */
export function ownerCreditsPayer(ownerId: string): CreditsPayer {
  return ownerId === currentCreditsActor() ? "self" : { org: { id: ownerId } };
}

/** Opt-in for a one-shot mutation on a resource owned by `ownerId`. */
export function mutationCreditsDenial(
  name: string,
  resourceId: string,
  ownerId: string,
  attemptNonce: string = creditsAttemptNonce(),
): CreditsDenialRequest {
  return {
    key: `op:${name}:${resourceId}:${attemptNonce}`,
    payer: ownerCreditsPayer(ownerId),
  };
}

/** A fresh key suffix for one-shot mutations. */
export function creditsAttemptNonce(): string {
  return crypto.randomUUID();
}

/**
 * Public, auth and hosted pages never present the dialog, nor does Billing
 * itself (the person is already where the dialog would send them). The
 * Nyxbot onboarding page is public but runs a foreground registration.
 */
export function isCreditsDialogSuppressed(path: string): boolean {
  if (path === "/billing") return true;
  if (path === "/nyxbot/onboarding") return false;
  return (
    isPublicPath(path) || path === "/login/code" || path.startsWith("/connect/")
  );
}

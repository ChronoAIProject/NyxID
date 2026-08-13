/**
 * NyxID's own auth rejections carry a NUMERIC `error_code` from
 * `backend/src/errors/mod.rs`: 1001 `unauthorized`, 2000
 * `authentication_failed`, 2001 `token_expired`, 2002 `mfa_required`.
 * Aevatar's rejections never carry that field, so its presence is the
 * discriminator between "your NyxID session is dead" and "the chat backend
 * rejected the identity NyxID sent".
 *
 * The distinction is load-bearing. The assistant transport opts every request
 * out of the global sign-out (`preserveSessionOn401`) on the assumption that a
 * 401 there is always the downstream's. When that assumption is wrong, an
 * expired session renders as "you are still signed in" — false, and it sends
 * the user to reconnect a service that is not the problem.
 */
const NYXID_AUTH_ERROR_CODES: ReadonlySet<number> = new Set([
  1001, 2000, 2001, 2002,
]);

export function isNyxIdSessionAuthFailure(errorCode: unknown): boolean {
  return typeof errorCode === "number" && NYXID_AUTH_ERROR_CODES.has(errorCode);
}

/** Copy for a 401 that NyxID itself raised: the user's session is gone. */
export const ASSISTANT_SESSION_EXPIRED_MESSAGE =
  "Your NyxID session has expired. Sign in again to keep chatting.";

/**
 * Copy for a 401 the chat backend raised. Deliberately does NOT tell the user
 * to reconnect anything: this is platform configuration on the `aevatar`
 * service row, which no end user can reach.
 */
export const ASSISTANT_UPSTREAM_AUTH_MESSAGE =
  "The chat backend rejected NyxID's credentials. Your NyxID session is unaffected; this needs a platform admin.";

export class AssistantTurnActiveError extends Error {
  constructor() {
    super("A turn is already active for this conversation.");
    this.name = "AssistantTurnActiveError";
  }
}

/**
 * A user-initiated Stop interrupted the operation. Callers treat this as an
 * expected outcome, not a failure — error toasts must suppress it.
 */
export class AssistantTurnCancelledError extends Error {
  constructor() {
    super("The approval request was stopped.");
    this.name = "AssistantTurnCancelledError";
  }
}

/**
 * The conversation does not exist for this transport: deleted (tombstoned),
 * never created, or a local `draft-*` placeholder whose in-memory mapping did
 * not survive a reload.
 * This is the transports' CONFIRMED not-found
 * sentinel — the page may treat it (alongside an HTTP 404) as license to
 * repair a stale `?c=` down to the draft state. Transient failures must stay
 * plain errors: repairing on those would turn a recoverable read into
 * navigation loss.
 */
export class AssistantConversationNotFoundError extends Error {
  constructor() {
    super("Conversation was not found.");
    this.name = "AssistantConversationNotFoundError";
  }
}

/**
 * Aevatar answered with a body that does not match any contract shape we
 * accept. Distinct from a transport failure on purpose: a transient blip may
 * fall back to a cached transcript, but a contract break must never be
 * laundered into "this chat has no messages" — that is exactly how the
 * array→`{messages, stateVersion}` change would have gone unnoticed.
 */
export class AssistantProtocolError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "AssistantProtocolError";
  }
}

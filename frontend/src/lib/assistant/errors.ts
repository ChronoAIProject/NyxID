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
 * never created, or a `nyxid-pending-*` placeholder whose in-memory mapping
 * did not survive a reload. This is the transports' CONFIRMED not-found
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

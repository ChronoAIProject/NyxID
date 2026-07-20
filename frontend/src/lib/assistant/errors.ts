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

export class AssistantTurnActiveError extends Error {
  constructor() {
    super("A turn is already active for this conversation.");
    this.name = "AssistantTurnActiveError";
  }
}

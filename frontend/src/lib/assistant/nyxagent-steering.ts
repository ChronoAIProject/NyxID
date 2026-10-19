/** Stable, safe copy shared by composer feedback and persisted transcript rows. */
export function steeringNotice(code: string | null, outcome?: string): string {
  if (outcome === "applied") return "Guidance applied to this reply.";
  switch (code) {
    case "starting": return "The assistant is starting. Try this guidance again shortly.";
    case "no_active_turn": return "This turn has ended. Send the guidance as a new message?";
    case "response_mismatch": return "The running response changed. Turn state refreshed; try your guidance again.";
    case "idempotency_conflict": return "This request ID was reused with different guidance. Reload before trying again.";
    case "steer_limit": return "This reply has reached its guidance limit. Wait for it to finish.";
    case "stop_pending": return "The assistant is stopping. Wait for it to finish.";
    case "steer_unsupported": return "Steering is not supported for this turn.";
    case "turn_active": return "Wait for this reply to finish before sending another message.";
    case "invalid_request": return "Guidance must be text only and at most 32,000 characters.";
    case "not_found": return "The assistant conversation was not found.";
    default: return "This guidance may not have been applied. It will not be sent again automatically.";
  }
}

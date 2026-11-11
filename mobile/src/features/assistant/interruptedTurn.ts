import type { AssistantHistory } from "./assistantTypes";

/** Only server-recorded replies prove settlement. A continuation names a card,
 * whose creation_turn_id links it to the request; unrelated replies prove nothing. */
export function settledInterruptedTurn(
  history: AssistantHistory | undefined,
  turnId?: string,
) {
  if (!history || !turnId) return undefined;
  const turns = new Set([turnId]);
  const cards = new Map(
    history.acknowledgements.map((card) => [card.id, card.creation_turn_id]),
  );
  // Follow chains even when older pages were merged out of arrival order.
  for (let changed = true; changed; ) {
    changed = false;
    for (const message of history.messages) {
      const parent =
        message.continuation_of && cards.get(message.continuation_of);
      if (parent && turns.has(parent) && !turns.has(message.turn_id)) {
        turns.add(message.turn_id);
        changed = true;
      }
    }
  }
  return history.messages
    .filter(
      (message) => message.role === "assistant" && turns.has(message.turn_id),
    )
    .reduce<
      AssistantHistory["messages"][number] | undefined
    >((latest, message) => (!latest || message.seq > latest.seq ? message : latest), undefined);
}

import type { CmaTranscriptTurn } from "@/schemas/nyxbot-cma-agent";
import type { CmaDisplayEvent } from "./cma-events";

/** Only an explicit, consistent CMA turn-start binding can locate a response. */
export function linkedCmaTurn(
  events: readonly CmaDisplayEvent[],
  responseId: string | undefined,
): number | undefined {
  if (!responseId) return undefined;
  const bound = events.filter((event) => event.responseId === responseId);
  const turns = new Set(bound.map((event) => event.turnSequence));
  if (turns.size !== 1) return undefined;
  const turn = [...turns][0];
  return turn &&
    bound.some(
      (event) => event.type === "turn_started" && event.sequence === turn,
    )
    ? turn
    : undefined;
}

export function cmaTurnEvents(
  turn: CmaTranscriptTurn,
  turns: readonly CmaTranscriptTurn[],
  events: readonly CmaDisplayEvent[],
): CmaDisplayEvent[] {
  const next = Math.min(
    ...turns
      .map((item) => item.turn_sequence)
      .filter((sequence) => sequence > turn.turn_sequence),
  );
  const bindings = new Set(
    events
      .filter(
        (event) =>
          event.type === "turn_started" &&
          event.sequence === turn.turn_sequence &&
          event.turnSequence === turn.turn_sequence &&
          event.responseId,
      )
      .map((event) => event.responseId),
  );
  if (bindings.size > 1) return [];
  const response = [...bindings][0];
  return events
    .filter(
      (event) =>
        event.sequence !== null &&
        event.sequence >= turn.turn_sequence &&
        event.sequence < next &&
        (event.turnSequence === undefined ||
          event.turnSequence === turn.turn_sequence) &&
        (!event.responseId || event.responseId === response),
    )
    .sort((a, b) => a.sequence! - b.sequence!);
}

export function cmaTurnStatus(
  turn: CmaTranscriptTurn,
  events: readonly CmaDisplayEvent[],
): string {
  if (turn.status !== "active") return turn.status;
  const status = events
    .filter((event) => event.type === "status_changed")
    .at(-1)?.status;
  return ["waiting_approval", "connection_required", "paused"].includes(
    status ?? "",
  )
    ? status!
    : "in_progress";
}

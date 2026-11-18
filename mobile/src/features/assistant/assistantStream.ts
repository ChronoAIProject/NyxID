import { turnEventSchema, type AssistantTurnEvent } from "./assistantTypes";

/** Incremental SSE framing, including split CRLF and multi-line data fields. */
export class AssistantSseDecoder {
  private buffer = "";
  private data: string[] = [];
  private frameSize = 0;

  push(chunk: string): AssistantTurnEvent[] {
    this.buffer += chunk;
    const events: AssistantTurnEvent[] = [];
    for (;;) {
      const index = this.buffer.search(/[\r\n]/);
      if (
        index < 0 ||
        (this.buffer[index] === "\r" && index === this.buffer.length - 1)
      )
        break;
      const line = this.buffer.slice(0, index);
      const length =
        this.buffer[index] === "\r" && this.buffer[index + 1] === "\n" ? 2 : 1;
      this.buffer = this.buffer.slice(index + length);
      this.frameSize += line.length;
      if (this.frameSize > 1024 * 1024)
        throw new Error("Assistant event exceeded its limit.");
      if (!line) {
        const payload = this.data.join("\n");
        this.data = [];
        this.frameSize = 0;
        if (!payload) continue;
        const parsed: unknown = JSON.parse(payload);
        if (!parsed || typeof parsed !== "object")
          throw new Error("Invalid assistant event.");
        const name = (parsed as { event?: unknown }).event;
        if (typeof name !== "string")
          throw new Error("Invalid assistant event.");
        if (!KNOWN_EVENTS.has(name)) continue;
        events.push(turnEventSchema.parse(parsed));
      } else if (line.startsWith("data:")) {
        this.data.push(line.slice(5).replace(/^ /, ""));
      }
    }
    if (this.buffer.length + this.frameSize > 1024 * 1024)
      throw new Error("Assistant event exceeded its limit.");
    return events;
  }
}
const KNOWN_EVENTS = new Set([
  "turn.status",
  "turn.continuing",
  "turn.notice",
  "message.started",
  "block.started",
  "block.delta",
  "block.completed",
  "message.completed",
  "turn.completed",
]);

export type AssistantLiveReply = {
  cursor: number;
  conversationId?: string;
  turnId?: string;
  messageId?: string;
  blocks: { id: string; index: number; text: string }[];
  terminal?: "completed" | "failed" | "cancelled";
  notice?: string;
  noticeCode?: string;
  continuations?: number;
  status?: Extract<AssistantTurnEvent, { event: "turn.status" }>;
  result?: Extract<AssistantTurnEvent, { event: "turn.completed" }>;
};
export const emptyLiveReply = (): AssistantLiveReply => ({
  cursor: 0,
  blocks: [],
});

/** Completed blocks replace deltas: the server's terminal text is authoritative. */
export function applyAssistantEvent(
  state: AssistantLiveReply,
  event: AssistantTurnEvent,
): AssistantLiveReply {
  if (event.cursor <= state.cursor) return state;
  if (
    event.event === "turn.status" &&
    state.conversationId &&
    event.conversation_id !== state.conversationId
  ) {
    throw new Error("Assistant conversation mismatch.");
  }
  if ("turn_id" in event && state.turnId && event.turn_id !== state.turnId) {
    throw new Error("Assistant turn mismatch.");
  }
  const next = { ...state, cursor: event.cursor };
  switch (event.event) {
    case "turn.status":
      return {
        ...next,
        conversationId: event.conversation_id,
        turnId: event.turn_id,
        status: event,
      };
    case "turn.notice":
      return { ...next, notice: event.message, noticeCode: event.code };
    case "turn.continuing":
      return { ...next, continuations: event.continuation };
    case "message.started":
      if (state.messageId && state.messageId !== event.message_id)
        throw new Error("Assistant message mismatch.");
      return { ...next, messageId: event.message_id };
    case "block.started": {
      if (
        event.message_id !== state.messageId ||
        event.block.block_id !== event.block_id
      )
        throw new Error("Assistant block identity mismatch.");
      if (state.blocks.some((item) => item.id === event.block_id))
        throw new Error("Duplicate assistant block.");
      return {
        ...next,
        blocks: [
          ...state.blocks,
          { id: event.block_id, index: event.index, text: event.block.text },
        ].sort((a, b) => a.index - b.index),
      };
    }
    case "block.delta":
    case "block.completed": {
      if (
        event.event === "block.completed" &&
        event.block.block_id !== event.block_id
      )
        throw new Error("Assistant block identity mismatch.");
      if (!state.blocks.some((item) => item.id === event.block_id))
        throw new Error("Assistant block was not started.");
      return {
        ...next,
        blocks: state.blocks.map((item) =>
          item.id === event.block_id
            ? {
                ...item,
                text:
                  event.event === "block.delta"
                    ? item.text + event.text
                    : event.block.text,
              }
            : item,
        ),
      };
    }
    case "turn.completed":
      return { ...next, terminal: event.status, result: event };
    case "message.completed":
      if (event.message_id !== state.messageId)
        throw new Error("Assistant message mismatch.");
      return next;
    default:
      return next;
  }
}

export function liveReplyText(state: AssistantLiveReply): string {
  return state.blocks.map((block) => block.text).join("\n\n");
}

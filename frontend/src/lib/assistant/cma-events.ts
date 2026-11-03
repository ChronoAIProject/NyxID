import {
  cmaPublicText,
  type CmaTranscriptMessage,
  type CmaResourceEvent,
} from "@/schemas/nyxbot-cma-agent";

export interface CmaEventScope {
  agentId: string;
  responseId?: string;
}
export interface CmaDisplayEvent {
  key: string;
  cursor?: string;
  sequence: number | null;
  type: string;
  message: string;
  status?: string;
  createdAt?: string;
  messageId?: string;
  messageSequence?: number;
}
function record(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}
function text(value: unknown): string | undefined {
  return typeof value === "string" ? value.slice(0, 128 * 1024) : undefined;
}
function sequence(value: unknown): number | null {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0
    ? value
    : null;
}

export function normalizeCmaEvent(
  raw: unknown,
  scope: CmaEventScope,
  eventId?: string,
): CmaDisplayEvent | null {
  const frame = record(raw);
  if (
    !frame ||
    (frame.agent_id !== undefined && frame.agent_id !== scope.agentId)
  )
    return null;
  if (scope.responseId) {
    const response = record(frame.response);
    const conversation = record(response?.conversation);
    if (
      (frame.response_id !== undefined &&
        frame.response_id !== scope.responseId) ||
      (response?.id !== undefined && response.id !== scope.responseId) ||
      (conversation?.id !== undefined && conversation.id !== scope.agentId)
    )
      return null;
    const type = text(frame.type);
    if (
      !type ||
      type.includes("delta") ||
      type.includes("content_part") ||
      type.includes("output_text") ||
      type === "response.output_item.added"
    )
      return null;
    if (!type.startsWith("response.") && type !== "error") return null;
    const seq = sequence(frame.sequence_number);
    const cursor = eventId ?? (seq === null ? undefined : String(seq));
    if (!cursor) return null;
    const item = record(frame.item);
    return {
      key: `response:${scope.responseId}:${cursor}`,
      cursor,
      sequence: seq,
      type,
      message:
        cmaPublicText(item?.content) ||
        text(item?.name) ||
        text(frame.message) ||
        "",
      status: text(response?.status) ?? text(item?.status),
      messageId: text(item?.id),
    };
  }
  const kind = record(frame.kind);
  const type = text(kind?.type) ?? text(frame.type);
  const seq = sequence(frame.sequence);
  const cursor = text(frame.cursor) ?? eventId ?? text(frame.id);
  if (!type || (!cursor && seq === null)) return null;
  return {
    key: `agent:${scope.agentId}:${cursor ?? seq}`,
    cursor,
    sequence: seq,
    type,
    message:
      cmaPublicText(kind?.content) ||
      text(kind?.title) ||
      text(frame.message) ||
      "",
    status: text(kind?.status) ?? text(kind?.to) ?? text(frame.status),
    createdAt: text(frame.appended_at) ?? text(frame.created_at),
    messageSequence:
      sequence(frame.message_sequence) ??
      (type === "message" ? (seq ?? undefined) : undefined),
  };
}

export function snapshotCmaEvents(
  events: readonly CmaResourceEvent[],
  agentId: string,
): CmaDisplayEvent[] {
  return events.flatMap((event) => {
    const normalized = normalizeCmaEvent(event, { agentId });
    return normalized ? [normalized] : [];
  });
}
export function mergeCmaEvents(
  before: readonly CmaDisplayEvent[],
  incoming: readonly CmaDisplayEvent[],
): CmaDisplayEvent[] {
  return [
    ...new Map(
      [...before, ...incoming].map((event) => [event.key, event]),
    ).values(),
  ].slice(-500);
}
export function orderCmaMessages(
  messages: readonly CmaTranscriptMessage[],
): CmaTranscriptMessage[] {
  const seen = new Set<string>();
  return messages
    .filter((message) => {
      const key =
        message.sequence !== null
          ? `sequence:${message.sequence}`
          : message.id
            ? `id:${message.id}`
            : undefined;
      if (!key) return true;
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .map((message, index) => ({ message, index }))
    .sort((a, b) =>
      a.message.sequence !== null && b.message.sequence !== null
        ? a.message.sequence - b.message.sequence
        : a.index - b.index,
    )
    .map(({ message }) => message);
}

export interface CmaSseFrame {
  id?: string;
  event?: string;
  data: unknown;
}
/** CMA is its own protocol. Never feed its frames to the NyxAgent tool reducer. */
export async function* readCmaStream(
  response: Response,
): AsyncGenerator<CmaSseFrame> {
  if (!response.body) throw new Error("CMA event stream is unavailable");
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";
  let received = 0;
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (done) break;
      received += value.byteLength;
      if (received > 8 * 1024 * 1024)
        throw new Error(
          "CMA stream byte limit reached. Reconnect to continue.",
        );
      buffer += decoder.decode(value, { stream: true });
      let match: RegExpExecArray | null;
      while ((match = /\r\n\r\n|\n\n|\r\r/.exec(buffer))) {
        const block = buffer.slice(0, match.index);
        buffer = buffer.slice(match.index + match[0].length);
        if (new TextEncoder().encode(block).byteLength > 64 * 1024)
          throw new Error("CMA event byte limit exceeded");
        const parts: string[] = [];
        let id: string | undefined;
        let event: string | undefined;
        for (const line of block.split(/\r\n|\n|\r/)) {
          if (line.startsWith("data:"))
            parts.push(line.slice(5).replace(/^ /, ""));
          if (line.startsWith("id:")) id = line.slice(3).replace(/^ /, "");
          if (line.startsWith("event:"))
            event = line.slice(6).replace(/^ /, "");
        }
        if (parts.length && parts.join("\n") !== "[DONE]")
          yield { id, event, data: JSON.parse(parts.join("\n")) as unknown };
      }
      if (new TextEncoder().encode(buffer).byteLength > 64 * 1024)
        throw new Error("CMA event buffer limit exceeded");
    }
  } finally {
    await reader.cancel().catch(() => undefined);
    reader.releaseLock();
  }
}

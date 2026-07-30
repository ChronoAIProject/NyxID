import { drainSseBuffer, flushSseBuffer } from "@/lib/assistant/sse";
import type { ChatStreamFrame } from "@/lib/assistant/chat-stream-worker-protocol";

function parseFrame(payload: string): ChatStreamFrame | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(payload);
  } catch {
    return null;
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    return null;
  }
  return parsed as ChatStreamFrame;
}

function parsePayloads(payloads: readonly string[]): ChatStreamFrame[] {
  const frames: ChatStreamFrame[] = [];
  for (const payload of payloads) {
    const frame = parseFrame(payload);
    if (frame) frames.push(frame);
  }
  return frames;
}

/** Incremental UTF-8/SSE parser shared by the worker and inline fallback. */
export class ChatStreamParser {
  private readonly decoder = new TextDecoder();
  private buffer = "";

  push(value: Uint8Array): ChatStreamFrame[] {
    this.buffer += this.decoder.decode(value, { stream: true });
    const { payloads, rest } = drainSseBuffer(this.buffer);
    this.buffer = rest;
    return parsePayloads(payloads);
  }

  finish(): ChatStreamFrame[] {
    this.buffer += this.decoder.decode();
    const frames = parsePayloads(flushSseBuffer(this.buffer));
    this.buffer = "";
    return frames;
  }
}

import type { AssistantWireLine } from "@/schemas/assistant-wire-log";

export interface WireLineFramerResult {
  readonly lines: readonly AssistantWireLine[];
  readonly bytes: number;
  readonly truncated: boolean;
}

/**
 * Replay-only raw line capture. It has a separate decoder and buffer from the
 * live SSE parser so capture cannot change frame parsing or delivery.
 */
export class WireLineFramer {
  private readonly decoder = new TextDecoder("utf-8", { fatal: false });
  private readonly maxRetainedBytes: number;
  private buffer = "";
  private receivedBytes = 0;
  private retainedInputBytes = 0;
  private isTruncated = false;
  private finished = false;

  constructor(maxRetainedBytes: number) {
    this.maxRetainedBytes = Math.max(0, maxRetainedBytes);
  }

  push(value: Uint8Array): WireLineFramerResult {
    if (this.finished)
      return { lines: [], bytes: 0, truncated: this.isTruncated };
    this.receivedBytes += value.byteLength;
    const remaining = Math.max(
      0,
      this.maxRetainedBytes - this.retainedInputBytes,
    );
    const retained = value.subarray(0, Math.min(value.byteLength, remaining));
    this.retainedInputBytes += retained.byteLength;
    if (retained.byteLength < value.byteLength) this.isTruncated = true;
    if (retained.byteLength > 0) {
      this.buffer += this.decoder.decode(retained, { stream: true });
    }
    return {
      lines: this.drain(false),
      bytes: value.byteLength,
      truncated: this.isTruncated,
    };
  }

  finish(): WireLineFramerResult {
    if (this.finished)
      return { lines: [], bytes: 0, truncated: this.isTruncated };
    this.finished = true;
    this.buffer += this.decoder.decode();
    return {
      lines: this.drain(true),
      bytes: 0,
      truncated: this.isTruncated,
    };
  }

  get bytes(): number {
    return this.receivedBytes;
  }

  get truncated(): boolean {
    return this.isTruncated;
  }

  private drain(final: boolean): AssistantWireLine[] {
    const lines: AssistantWireLine[] = [];
    let lineStart = 0;
    let index = 0;
    while (index < this.buffer.length) {
      const character = this.buffer[index];
      if (character === "\n") {
        lines.push({ text: this.buffer.slice(lineStart, index), ending: "\n" });
        index += 1;
        lineStart = index;
        continue;
      }
      if (character !== "\r") {
        index += 1;
        continue;
      }
      if (index + 1 >= this.buffer.length && !final) break;
      const hasLineFeed = this.buffer[index + 1] === "\n";
      lines.push({
        text: this.buffer.slice(lineStart, index),
        ending: hasLineFeed ? "\r\n" : "\r",
      });
      index += hasLineFeed ? 2 : 1;
      lineStart = index;
    }
    this.buffer = this.buffer.slice(lineStart);
    if (final && this.buffer.length > 0) {
      lines.push({ text: this.buffer, ending: "" });
      this.buffer = "";
    }
    return lines;
  }
}

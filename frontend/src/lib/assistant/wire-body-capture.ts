export interface WireBodyCaptureResult {
  readonly text: string;
  readonly bytes: number;
  readonly truncated: boolean;
}

export class WireBodyCapture {
  private readonly decoder = new TextDecoder("utf-8", { fatal: false });
  private readonly maxRetainedBytes: number;
  private readonly parts: string[] = [];
  private receivedBytes = 0;
  private retainedInputBytes = 0;
  private isTruncated = false;
  private finished = false;

  constructor(maxRetainedBytes: number) {
    this.maxRetainedBytes = Math.max(0, maxRetainedBytes);
  }

  push(value: Uint8Array): void {
    if (this.finished) return;
    this.receivedBytes += value.byteLength;
    const remaining = Math.max(
      0,
      this.maxRetainedBytes - this.retainedInputBytes,
    );
    const retained = value.subarray(0, Math.min(value.byteLength, remaining));
    this.retainedInputBytes += retained.byteLength;
    if (retained.byteLength < value.byteLength) this.isTruncated = true;
    if (retained.byteLength > 0) {
      this.parts.push(this.decoder.decode(retained, { stream: true }));
    }
  }

  finish(): WireBodyCaptureResult {
    if (!this.finished) {
      this.finished = true;
      this.parts.push(this.decoder.decode());
    }
    return {
      text: this.parts.join(""),
      bytes: this.receivedBytes,
      truncated: this.isTruncated,
    };
  }

  get truncated(): boolean {
    return this.isTruncated;
  }
}

export function isSseMediaType(contentType: string): boolean {
  return (
    (contentType.split(";", 1)[0] ?? "").trim().toLowerCase() ===
    "text/event-stream"
  );
}

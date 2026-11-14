import { fromByteArray } from "base64-js";

export const TERMINAL_OUTPUT_BATCH_BYTES = 64 * 1024;
export const TERMINAL_OUTPUT_QUEUE_BYTES = 4 * 1024 * 1024;

// One acknowledged batch at a time bounds both the JS queue and WebView work.
export class TerminalOutput {
  private queue: Uint8Array[] = [];
  private bytes = 0;
  private gapPending = false;
  private sequence = 0;
  private awaiting: number | undefined;
  private timer: ReturnType<typeof setTimeout> | undefined;
  constructor(
    private send: (data: string, id: number) => void,
    private marker: () => Uint8Array,
  ) {}
  push(data: ArrayBuffer) {
    if (data.byteLength === 0) return;
    let incoming = new Uint8Array(data);
    const markerBytes = this.marker().byteLength;
    const reservedMarkerBytes = this.gapPending ? markerBytes : 0;
    if (
      this.bytes + incoming.byteLength + reservedMarkerBytes >
      TERMINAL_OUTPUT_QUEUE_BYTES
    ) {
      this.gapPending = true;
      const capacity = TERMINAL_OUTPUT_QUEUE_BYTES - markerBytes;
      if (incoming.byteLength > capacity)
        incoming = incoming.subarray(incoming.byteLength - capacity);
      while (this.bytes + incoming.byteLength > capacity && this.queue.length) {
        const head = this.queue[0]!;
        const drop = Math.min(head.byteLength, this.bytes + incoming.byteLength - capacity);
        if (drop === head.byteLength) this.queue.shift();
        else this.queue[0] = head.subarray(drop);
        this.bytes -= drop;
      }
    }
    this.queue.push(incoming);
    this.bytes += incoming.byteLength;
    this.schedule();
  }
  ack(id: number) {
    if (id !== this.awaiting) return;
    this.awaiting = undefined;
    this.schedule();
  }
  private schedule() {
    if (this.timer || this.awaiting || !this.queue.length) return;
    this.timer = setTimeout(() => {
      this.timer = undefined;
      this.flush();
    }, 16);
  }
  private flush() {
    if (this.awaiting || !this.queue.length) return;
    const marker = this.gapPending ? this.marker() : new Uint8Array();
    this.gapPending = false;
    const batch = new Uint8Array(
      Math.min(this.bytes + marker.byteLength, TERMINAL_OUTPUT_BATCH_BYTES),
    );
    batch.set(marker);
    let offset = marker.byteLength;
    while (offset < batch.length && this.queue.length) {
      const head = this.queue[0]!;
      const take = Math.min(head.length, batch.length - offset);
      batch.set(head.subarray(0, take), offset);
      offset += take;
      if (take === head.length) this.queue.shift();
      else this.queue[0] = head.subarray(take);
    }
    this.bytes -= batch.length - marker.byteLength;
    const id = ++this.sequence;
    this.awaiting = id;
    this.send(fromByteArray(batch), id);
  }
  clear() {
    clearTimeout(this.timer);
    this.timer = undefined;
    this.queue = [];
    this.bytes = 0;
    this.awaiting = undefined;
    this.gapPending = false;
  }
  get queuedBytes() {
    return this.bytes + (this.gapPending ? this.marker().byteLength : 0);
  }
}

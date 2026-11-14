import { fromByteArray } from "base64-js";

// One acknowledged batch at a time bounds both the JS queue and WebView work.
export class TerminalOutput {
  private queue: Uint8Array[] = [];
  private bytes = 0;
  private sequence = 0;
  private awaiting: number | undefined;
  private timer: ReturnType<typeof setTimeout> | undefined;
  constructor(
    private send: (data: string, id: number) => void,
    private overflow: () => void,
  ) {}
  push(data: ArrayBuffer) {
    if (this.bytes + data.byteLength > 262144) {
      this.clear();
      this.overflow();
      return;
    }
    this.queue.push(new Uint8Array(data));
    this.bytes += data.byteLength;
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
    const batch = new Uint8Array(Math.min(this.bytes, 16384));
    let offset = 0;
    while (offset < batch.length && this.queue.length) {
      const head = this.queue[0]!;
      const take = Math.min(head.length, batch.length - offset);
      batch.set(head.subarray(0, take), offset);
      offset += take;
      if (take === head.length) this.queue.shift();
      else this.queue[0] = head.subarray(take);
    }
    this.bytes -= batch.length;
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
  }
}

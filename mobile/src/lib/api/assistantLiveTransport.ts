import { z } from "zod";
import { ApiError } from "./ApiError";

const eventSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("ready") }),
  z.object({
    type: z.literal("conversation"),
    id: z.string(),
    group_id: z.string().nullable().default(null),
    turn_id: z.string().nullable().default(null),
    messages: z.number().int().nonnegative().default(0),
    title_changed: z.boolean().optional(),
  }),
  z.object({ type: z.literal("group"), id: z.string() }),
  z.object({ type: z.literal("channels") }),
  z.object({
    type: z.literal("channel_thread"),
    id: z.string(),
    channel_id: z.string(),
    parent_id: z.string().nullable(),
    conversation_id: z.string().nullable(),
  }),
  z.object({ type: z.literal("resync") }),
]);
export type AssistantLiveEvent = z.infer<typeof eventSchema>;
export type AssistantLiveStatus =
  | "idle"
  | "connecting"
  | "live"
  | "unsupported";
export const ASSISTANT_LIVE_BACKOFF_MS = [
  1000, 2000, 5000, 10_000, 30_000,
] as const;
export const ASSISTANT_LIVE_UNSUPPORTED_RETRY_MS = 300_000;
export const ASSISTANT_LIVE_REOPEN_MS = 250;
export const ASSISTANT_LIVE_MAX_FRAME_CHARS = 64 * 1024;
export const ASSISTANT_LIVE_MAX_CHUNK_BYTES = 256 * 1024;
const MAX_CONNECTION_BYTES = 8 * 1024 * 1024;

export interface AssistantLiveClock {
  now: () => number;
  setTimeout: (callback: () => void, milliseconds: number) => unknown;
  clearTimeout: (handle: unknown) => void;
}
export interface AssistantLiveTiming {
  connectTimeoutMs: number;
  readyTimeoutMs: number;
  heartbeatTimeoutMs: number;
}
const systemClock: AssistantLiveClock = {
  now: () => Date.now(),
  setTimeout: (callback, milliseconds) => setTimeout(callback, milliseconds),
  clearTimeout: (handle) =>
    clearTimeout(handle as ReturnType<typeof setTimeout>),
};
const DEFAULT_TIMING: AssistantLiveTiming = {
  connectTimeoutMs: 15_000,
  readyTimeoutMs: 15_000,
  heartbeatTimeoutMs: 45_000,
};
type LiveResponse = {
  headers: { get: (name: string) => string | null };
  body: ReadableStream<Uint8Array> | null;
};

/** SSE lines can end in LF, CRLF, or CR, including across byte chunks. */
class LiveFrames {
  private line = "";
  private data: string[] = [];
  private size = 0;
  private afterCr = false;

  push(text: string): string[] {
    const payloads: string[] = [];
    for (const character of text) {
      if (this.afterCr) {
        this.afterCr = false;
        if (character === "\n") continue;
      }
      this.size += character.length;
      if (this.size > ASSISTANT_LIVE_MAX_FRAME_CHARS)
        throw new Error("Assistant live frame exceeded its limit.");
      if (character === "\r" || character === "\n") {
        if (!this.line) {
          if (this.data.length) payloads.push(this.data.join("\n"));
          this.data = [];
          this.size = 0;
        } else {
          const colon = this.line.indexOf(":");
          const field = colon < 0 ? this.line : this.line.slice(0, colon);
          let value = colon < 0 ? "" : this.line.slice(colon + 1);
          if (value.startsWith(" ")) value = value.slice(1);
          if (field === "data") this.data.push(value);
        }
        this.line = "";
        this.afterCr = character === "\r";
      } else this.line += character;
    }
    return payloads;
  }
}

function parseEvent(payload: string): AssistantLiveEvent | undefined {
  try {
    const result = eventSchema.safeParse(JSON.parse(payload));
    return result.success ? result.data : undefined;
  } catch {
    return undefined;
  }
}
const aborted = () =>
  Object.assign(new Error("Assistant live request stopped."), {
    name: "AbortError",
  });

/** Race stuck native operations too: abort alone need not settle their Promise. */
function deadline<T>(
  operation: () => Promise<T>,
  milliseconds: number,
  controller: AbortController,
  clock: AssistantLiveClock,
): Promise<T> {
  return new Promise((resolve, reject) => {
    let finished = false;
    let timer: unknown;
    const finish = (callback: () => void) => {
      if (finished) return;
      finished = true;
      clock.clearTimeout(timer);
      controller.signal.removeEventListener("abort", stop);
      callback();
    };
    const stop = () => finish(() => reject(aborted()));
    controller.signal.addEventListener("abort", stop, { once: true });
    if (controller.signal.aborted) {
      stop();
      return;
    }
    timer = clock.setTimeout(() => {
      finish(() => reject(new Error("Assistant live request timed out.")));
      controller.abort();
    }, milliseconds);
    Promise.resolve()
      .then(() => {
        if (controller.signal.aborted) throw aborted();
        return operation();
      })
      .then(
        (value) => finish(() => resolve(value)),
        (error: unknown) => finish(() => reject(error)),
      );
  });
}

function wait(
  milliseconds: number,
  signal: AbortSignal,
  clock: AssistantLiveClock,
): Promise<void> {
  return new Promise((resolve) => {
    let timer: unknown;
    const finish = () => {
      clock.clearTimeout(timer);
      signal.removeEventListener("abort", finish);
      resolve();
    };
    signal.addEventListener("abort", finish, { once: true });
    if (signal.aborted) finish();
    else timer = clock.setTimeout(finish, milliseconds);
  });
}

/** One caller-owned stream; status/events stop when its outer signal aborts.
 * The owner resets its status to idle when releasing the subscription. */
export async function runAssistantLive({
  connect,
  signal,
  onStatus,
  onEvent,
  clock = systemClock,
  timing: overrides,
}: {
  connect: (signal: AbortSignal) => Promise<LiveResponse>;
  signal: AbortSignal;
  onStatus: (status: AssistantLiveStatus) => void;
  onEvent: (event: AssistantLiveEvent) => void;
  clock?: AssistantLiveClock;
  timing?: Partial<AssistantLiveTiming>;
}): Promise<void> {
  const timing = { ...DEFAULT_TIMING, ...overrides };
  for (const value of Object.values(timing)) {
    if (!Number.isFinite(value) || value <= 0)
      throw new Error("Assistant live deadlines must be positive.");
  }
  const state: { status: AssistantLiveStatus } = { status: "idle" };
  const setStatus = (next: AssistantLiveStatus) => {
    if (!signal.aborted && state.status !== next) {
      state.status = next;
      onStatus(next);
    }
  };
  let attempt = 0;
  while (!signal.aborted) {
    if (state.status !== "unsupported") setStatus("connecting");
    const controller = new AbortController();
    const stop = () => controller.abort();
    signal.addEventListener("abort", stop, { once: true });
    if (signal.aborted) stop();
    let response: LiveResponse | undefined;
    let reader: ReadableStreamDefaultReader<Uint8Array> | undefined;
    let streamSettled = false;
    let delay = ASSISTANT_LIVE_REOPEN_MS;
    try {
      response = await deadline(
        () => connect(controller.signal),
        timing.connectTimeoutMs,
        controller,
        clock,
      );
      if (signal.aborted) break;
      const contentType = response.headers
        .get("content-type")
        ?.split(";")[0]
        ?.trim()
        .toLowerCase();
      if (contentType !== "text/event-stream") {
        setStatus("unsupported");
        delay = ASSISTANT_LIVE_UNSUPPORTED_RETRY_MS;
      } else {
        if (!response.body)
          throw new Error("Assistant live stream returned no body.");
        reader = response.body.getReader();
        const frames = new LiveFrames();
        const decoder = new TextDecoder("utf-8", { fatal: true });
        const readyBy = clock.now() + timing.readyTimeoutMs;
        let heartbeatBy = clock.now() + timing.heartbeatTimeoutMs;
        let ready = false;
        let total = 0;
        let emptyReads = 0;
        while (!signal.aborted) {
          const timeout = Math.max(
            0,
            Math.min(
              heartbeatBy - clock.now(),
              ready ? Infinity : readyBy - clock.now(),
            ),
          );
          const result = await deadline(
            () =>
              reader!.read().catch((error: unknown) => {
                streamSettled = true;
                throw error;
              }),
            timeout,
            controller,
            clock,
          );
          if (signal.aborted) break;
          if (result.done) {
            streamSettled = true;
            decoder.decode();
            break;
          }
          if (!result.value.byteLength) {
            if (++emptyReads > 64)
              throw new Error(
                "Assistant live stream returned too many empty chunks.",
              );
            continue;
          }
          emptyReads = 0;
          heartbeatBy = clock.now() + timing.heartbeatTimeoutMs;
          total += result.value.byteLength;
          if (
            result.value.byteLength > ASSISTANT_LIVE_MAX_CHUNK_BYTES ||
            total > MAX_CONNECTION_BYTES
          )
            throw new Error("Assistant live response exceeded its limit.");
          for (const payload of frames.push(
            decoder.decode(result.value, { stream: true }),
          )) {
            if (signal.aborted) break;
            const event = parseEvent(payload);
            if (!event) continue;
            if (event.type === "ready") {
              ready = true;
              attempt = 0;
              setStatus("live");
            }
            if (!signal.aborted) onEvent(event);
          }
        }
        if (!ready && !signal.aborted)
          throw new Error("Assistant live stream closed before ready.");
        // EOF immediately restores fallback polling during the reopen gap.
        setStatus("connecting");
      }
    } catch (error) {
      if (signal.aborted) break;
      if (
        error instanceof ApiError &&
        [404, 405, 501].includes(error.statusCode)
      ) {
        setStatus("unsupported");
        delay = ASSISTANT_LIVE_UNSUPPORTED_RETRY_MS;
      } else {
        setStatus("connecting");
        delay =
          ASSISTANT_LIVE_BACKOFF_MS[
            Math.min(attempt, ASSISTANT_LIVE_BACKOFF_MS.length - 1)
          ] ?? ASSISTANT_LIVE_BACKOFF_MS[0];
        attempt++;
      }
    } finally {
      signal.removeEventListener("abort", stop);
      // Request abort owns native cancellation. reader.cancel() can race
      // Expo's didComplete close and must never be used here.
      if (response && !streamSettled && !controller.signal.aborted)
        controller.abort();
      reader?.releaseLock();
    }
    if (!signal.aborted) await wait(delay, signal, clock);
  }
}

export function livePollInterval(
  interval: number | false,
  live: boolean,
): number | false {
  return interval === false || !live ? interval : Math.max(interval, 30_000);
}

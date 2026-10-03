import { z } from "zod";
import { ApiError } from "@/lib/api-client";
import { assistantHttp } from "@/lib/assistant/assistant-http";
import { drainDirectSseBuffer } from "@/lib/assistant/direct-transport";

/**
 * NyxID pushes the owner's assistant changes over one server-sent event
 * stream (`GET /assistant/nyxagent/live`), fed by a MongoDB change stream:
 * identifiers only, never content. Views refresh when told instead of
 * polling; polling remains only as a slow backstop, and as the fallback when
 * the stream is unsupported (404 from an older server).
 */
export const NYXAGENT_LIVE_PATH = "/assistant/nyxagent/live";

const liveEventSchema = z.discriminatedUnion("type", [
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
  z.object({ type: z.literal("resync") }),
]);
export type NyxAgentLiveEvent = z.infer<typeof liveEventSchema>;

/** `live` once the stream is open; `unsupported` stops retrying. */
export type NyxAgentLiveStatus = "idle" | "connecting" | "live" | "unsupported";

/** Reconnect delays after errors; a stream the server closes reopens at once. */
export const NYXAGENT_LIVE_BACKOFF_MS = [1000, 2000, 5000, 10_000, 30_000] as const;
/** An unsupported answer (e.g. an older replica mid-deploy) is retried this late. */
export const NYXAGENT_LIVE_UNSUPPORTED_RETRY_MS = 5 * 60_000;
const REOPEN_MS = 250;

let status: NyxAgentLiveStatus = "idle";
const statusListeners = new Set<() => void>();

export function getNyxAgentLiveStatus(): NyxAgentLiveStatus {
  return status;
}

export function subscribeNyxAgentLiveStatus(listener: () => void): () => void {
  statusListeners.add(listener);
  return () => statusListeners.delete(listener);
}

function setStatus(next: NyxAgentLiveStatus) {
  if (status === next) return;
  status = next;
  for (const listener of statusListeners) listener();
}

export function parseNyxAgentLiveEvent(payload: string): NyxAgentLiveEvent | undefined {
  try {
    const parsed = liveEventSchema.safeParse(JSON.parse(payload));
    return parsed.success ? parsed.data : undefined;
  } catch {
    return undefined;
  }
}

function wait(ms: number, signal: AbortSignal) {
  return new Promise<void>((resolve) => {
    const timer = setTimeout(resolve, ms);
    signal.addEventListener(
      "abort",
      () => {
        clearTimeout(timer);
        resolve();
      },
      { once: true },
    );
  });
}

let connection: { readonly controller: AbortController; users: number } | undefined;
const eventListeners = new Set<(event: NyxAgentLiveEvent) => void>();

/**
 * Receive live events. The first listener opens the one shared stream; the
 * last one to leave closes it.
 */
export function listenNyxAgentLive(listener: (event: NyxAgentLiveEvent) => void): () => void {
  eventListeners.add(listener);
  if (!connection) {
    const controller = new AbortController();
    connection = { controller, users: 0 };
    void run(controller.signal);
  }
  connection.users += 1;
  let closed = false;
  return () => {
    if (closed) return;
    closed = true;
    eventListeners.delete(listener);
    if (connection && --connection.users === 0) {
      connection.controller.abort();
      connection = undefined;
      setStatus("idle");
    }
  };
}

function deliver(event: NyxAgentLiveEvent) {
  for (const listener of eventListeners) listener(event);
}

async function run(signal: AbortSignal) {
  let attempt = 0;
  while (!signal.aborted) {
    if (status !== "live" && status !== "unsupported") setStatus("connecting");
    try {
      const response = await assistantHttp(NYXAGENT_LIVE_PATH, {
        headers: { Accept: "text/event-stream" },
        signal,
      });
      if (!response.headers.get("content-type")?.includes("text/event-stream")) {
        // Something other than the live stream answered (an older server).
        setStatus("unsupported");
        await wait(NYXAGENT_LIVE_UNSUPPORTED_RETRY_MS, signal);
        continue;
      }
      if (!response.body) throw new Error("The live stream returned no body.");
      const reader = response.body.getReader();
      let ready = false;
      const cancel = () => void reader.cancel().catch(() => undefined);
      signal.addEventListener("abort", cancel, { once: true });
      const decoder = new TextDecoder();
      let buffer = "";
      try {
        for (;;) {
          const { done, value } = await reader.read();
          if (done || signal.aborted) break;
          buffer += decoder.decode(value, { stream: true });
          const drained = drainDirectSseBuffer(buffer);
          buffer = drained.rest;
          for (const payload of drained.payloads) {
            const event = parseNyxAgentLiveEvent(payload);
            if (!event) continue;
            if (event.type === "ready") {
              ready = true;
              attempt = 0;
              setStatus("live");
            }
            deliver(event);
          }
        }
      } finally {
        signal.removeEventListener("abort", cancel);
      }
      if (!ready && !signal.aborted) throw new Error("The live stream closed before it was ready.");
      // The server closes streams after a while: reopen promptly. The next
      // `ready` makes views re-read anything that changed in between.
      await wait(REOPEN_MS, signal);
    } catch (error) {
      if (signal.aborted) return;
      if (error instanceof ApiError && [404, 405, 501].includes(error.status)) {
        // Keep polling; a rolling deploy may bring the stream later.
        setStatus("unsupported");
        await wait(NYXAGENT_LIVE_UNSUPPORTED_RETRY_MS, signal);
        continue;
      }
      setStatus("connecting");
      const delay =
        NYXAGENT_LIVE_BACKOFF_MS[Math.min(attempt, NYXAGENT_LIVE_BACKOFF_MS.length - 1)] ??
        NYXAGENT_LIVE_BACKOFF_MS[0];
      attempt += 1;
      await wait(delay, signal);
    }
  }
}

/** Test hook: forget the shared stream and status. */
export function resetNyxAgentLiveForTests() {
  connection?.controller.abort();
  connection = undefined;
  eventListeners.clear();
  status = "idle";
}

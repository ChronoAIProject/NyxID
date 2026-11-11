import { AssistantSseDecoder } from "./assistantStream";
import type { AssistantTurnEvent } from "./assistantTypes";

/** Drain the terminal response to EOF so Expo's native completion owns close.
 * Canceling its reader first can race a queued didComplete controller.close(). */
export async function readAssistantResponse(
  body: ReadableStream<Uint8Array>,
  onEvent: (event: AssistantTurnEvent) => void,
  controller: AbortController,
): Promise<void> {
  const reader = body.getReader();
  const decoder = new TextDecoder("utf-8", { fatal: true });
  const frames = new AssistantSseDecoder();
  let terminal = false;
  let streamSettled = false;
  let total = 0;
  try {
    for (;;) {
      let result: ReadableStreamReadResult<Uint8Array>;
      try {
        result = await readWithDeadline(
          reader,
          terminal ? 5000 : 135000,
          controller,
        );
      } catch (cause) {
        // A rejected read means the stream has already errored (a deadline
        // aborts first). Do not cancel an already settled native response.
        streamSettled = true;
        // A validated terminal event is authoritative even if EOF is delayed
        // or the app leaves during transport cleanup. Do not suggest a resend.
        if (!terminal) throw cause;
        break;
      }
      if (result.done) {
        streamSettled = true;
        break;
      }
      total += result.value.byteLength;
      if (total > 12 * 1024 * 1024) {
        if (terminal) break;
        throw new Error("Assistant response exceeded its limit.");
      }
      if (terminal) continue;
      for (const event of frames.push(
        decoder.decode(result.value, { stream: true }),
      )) {
        if (terminal) break;
        onEvent(event);
        if (event.event === "turn.completed") terminal = true;
      }
    }
    if (!terminal)
      throw new Error(
        "The connection was interrupted. The assistant may still be working.",
      );
  } finally {
    // One cancellation owner: request abort errors the native stream. Calling
    // reader.cancel() as well can close it before a queued native completion.
    if (!streamSettled && !controller.signal.aborted) controller.abort();
    reader.releaseLock();
  }
}

async function readWithDeadline(
  reader: ReadableStreamDefaultReader<Uint8Array>,
  milliseconds: number,
  controller: AbortController,
): Promise<ReadableStreamReadResult<Uint8Array>> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      reader.read(),
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => {
          controller.abort();
          reject(
            new Error(
              "The connection was interrupted. Refresh history before sending again.",
            ),
          );
        }, milliseconds);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

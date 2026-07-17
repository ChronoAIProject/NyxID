/**
 * Incremental SSE framing shared by the assistant's streaming transports
 * (aevatar nyxid-chat AG-UI frames and the OpenAI-compatible completions
 * chunks). Both read the same `data:`-framed wire format; only the payload
 * shape differs.
 */

/**
 * Consumes complete `data:` payloads from the buffer and returns the
 * unterminated remainder for the next read.
 *
 * Line endings are normalized for all three SSE-legal forms (`\r\n`, `\n`,
 * and bare `\r`). A trailing `\r` is held back undecided: it may be half of
 * a `\r\n` split across network reads, and eagerly converting it to `\n`
 * would fabricate a `\n\n` frame boundary when the matching `\n` arrives
 * (splitting one multi-`data:`-line event into two payloads).
 */
export function drainSseBuffer(buffer: string): {
  readonly payloads: string[];
  readonly rest: string;
} {
  let working = buffer;
  let heldCr = "";
  if (working.endsWith("\r")) {
    heldCr = "\r";
    working = working.slice(0, -1);
  }
  const normalized = working.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
  const segments = normalized.split("\n\n");
  const rest = segments.pop() ?? "";
  const payloads: string[] = [];
  for (const segment of segments) {
    const data = segment
      .split("\n")
      .filter((line) => line.startsWith("data:"))
      .map((line) => line.slice("data:".length).trimStart())
      .join("\n");
    if (data) payloads.push(data);
  }
  return { payloads, rest: rest + heldCr };
}

/**
 * Flushes the final, unterminated SSE frame at end of stream. Servers may
 * close the connection right after the last `data:` line without a trailing
 * blank line; dropping that frame loses terminal events (`RUN_FINISHED`),
 * which would misreport a completed run as truncated.
 */
export function flushSseBuffer(rest: string): string[] {
  if (!rest.trim()) return [];
  return drainSseBuffer(`${rest}\n\n`).payloads;
}

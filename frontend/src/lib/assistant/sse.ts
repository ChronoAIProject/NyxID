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
 * and bare `\r`). Normalizing a trailing `\r` that later turns out to be
 * half of a split `\r\n` is safe: the reassembled buffer still produces the
 * same `\n\n` frame boundary on the next drain.
 */
export function drainSseBuffer(buffer: string): {
  readonly payloads: string[];
  readonly rest: string;
} {
  const normalized = buffer.replace(/\r\n/g, "\n").replace(/\r/g, "\n");
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
  return { payloads, rest };
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

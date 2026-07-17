/**
 * Incremental SSE framing shared by the assistant's streaming transports
 * (aevatar nyxid-chat AG-UI frames and the OpenAI-compatible completions
 * chunks). Both read the same `data:`-framed wire format; only the payload
 * shape differs.
 */

/**
 * Consumes complete `data:` payloads from the buffer and returns the
 * unterminated remainder for the next read.
 */
export function drainSseBuffer(buffer: string): {
  readonly payloads: string[];
  readonly rest: string;
} {
  const normalized = buffer.replace(/\r\n/g, "\n");
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

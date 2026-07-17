import { describe, expect, it } from "vitest";
import { drainSseBuffer } from "@/lib/assistant/sse";

describe("drainSseBuffer", () => {
  it("extracts complete data payloads and keeps the unterminated tail", () => {
    const { payloads, rest } = drainSseBuffer(
      'data: {"a":1}\n\ndata: {"b":2}\n\ndata: {"partial"',
    );
    expect(payloads).toEqual(['{"a":1}', '{"b":2}']);
    expect(rest).toBe('data: {"partial"');
  });

  it("handles CRLF framing and non-data lines", () => {
    const { payloads, rest } = drainSseBuffer(
      'event: message\r\ndata: {"a":1}\r\n\r\n',
    );
    expect(payloads).toEqual(['{"a":1}']);
    expect(rest).toBe("");
  });

  it("passes the non-JSON completions sentinel through as a payload", () => {
    const { payloads, rest } = drainSseBuffer("data: [DONE]\n\n");
    expect(payloads).toEqual(["[DONE]"]);
    expect(rest).toBe("");
  });
});

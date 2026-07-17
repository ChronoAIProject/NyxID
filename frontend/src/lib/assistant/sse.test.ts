import { describe, expect, it } from "vitest";
import { drainSseBuffer, flushSseBuffer } from "@/lib/assistant/sse";

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

  it("handles bare-CR framing, including a CRLF split across reads", () => {
    const lone = drainSseBuffer('data: {"a":1}\r\rdata: {"partial"');
    expect(lone.payloads).toEqual(['{"a":1}']);

    // A `\r\n\r\n` boundary split right after the first `\r`: the first
    // drain keeps the tail, the second read completes the boundary.
    const first = drainSseBuffer('data: {"a":1}\r');
    expect(first.payloads).toEqual([]);
    const second = drainSseBuffer(`${first.rest}\n\r\ndata: {"b":2}\r\n\r\n`);
    expect(second.payloads).toEqual(['{"a":1}', '{"b":2}']);
    expect(second.rest).toBe("");
  });
});

describe("flushSseBuffer", () => {
  it("recovers a final frame with no trailing blank line", () => {
    const { payloads, rest } = drainSseBuffer(
      'data: {"a":1}\n\ndata: {"type":"RUN_FINISHED"}',
    );
    expect(payloads).toEqual(['{"a":1}']);
    expect(flushSseBuffer(rest)).toEqual(['{"type":"RUN_FINISHED"}']);
  });

  it("returns nothing for whitespace-only remainders", () => {
    expect(flushSseBuffer("")).toEqual([]);
    expect(flushSseBuffer("\n")).toEqual([]);
  });
});

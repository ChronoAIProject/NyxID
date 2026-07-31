import { describe, expect, it } from "vitest";
import { WireLineFramer } from "./wire-line-framer";

const encoder = new TextEncoder();

describe("WireLineFramer", () => {
  it("preserves SSE fields, comments, blank separators, and exact endings", () => {
    const framer = new WireLineFramer(1024);

    expect(
      framer.push(
        encoder.encode(
          "event: turn\nid: 7\r\ndata: first\r: keepalive\n\ndata: tail",
        ),
      ).lines,
    ).toEqual([
      { text: "event: turn", ending: "\n" },
      { text: "id: 7", ending: "\r\n" },
      { text: "data: first", ending: "\r" },
      { text: ": keepalive", ending: "\n" },
      { text: "", ending: "\n" },
    ]);
    expect(framer.finish().lines).toEqual([{ text: "data: tail", ending: "" }]);
  });

  it("waits for a CRLF split across chunks before choosing the ending", () => {
    const framer = new WireLineFramer(1024);

    expect(framer.push(encoder.encode("data: one\r")).lines).toEqual([]);
    expect(framer.push(encoder.encode("\ndata: two\rnext")).lines).toEqual([
      { text: "data: one", ending: "\r\n" },
      { text: "data: two", ending: "\r" },
    ]);
    expect(framer.finish().lines).toEqual([{ text: "next", ending: "" }]);
  });

  it("decodes a multibyte character split across chunks", () => {
    const framer = new WireLineFramer(1024);
    const bytes = encoder.encode("data: hé\n");
    const split = bytes.indexOf(0xc3) + 1;

    expect(framer.push(bytes.slice(0, split)).lines).toEqual([]);
    expect(framer.push(bytes.slice(split)).lines).toEqual([
      { text: "data: hé", ending: "\n" },
    ]);
  });

  it("uses replacement characters for invalid UTF-8", () => {
    const framer = new WireLineFramer(1024);

    expect(
      framer.push(
        Uint8Array.from([0x64, 0x61, 0x74, 0x61, 0x3a, 0x20, 0xff, 0x0a]),
      ).lines,
    ).toEqual([{ text: "data: �", ending: "\n" }]);
  });

  it("counts all received bytes while retaining only the capped prefix", () => {
    const framer = new WireLineFramer(8);

    const first = framer.push(encoder.encode("data: 1\nextra"));
    expect(first).toMatchObject({ bytes: 13, truncated: true });
    expect(first.lines).toEqual([{ text: "data: 1", ending: "\n" }]);
    expect(framer.push(encoder.encode(" ignored")).lines).toEqual([]);
    expect(framer.finish()).toMatchObject({
      lines: [],
      bytes: 0,
      truncated: true,
    });
    expect(framer.bytes).toBe(21);
  });
});

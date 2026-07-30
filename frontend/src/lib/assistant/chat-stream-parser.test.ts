import { describe, expect, it } from "vitest";
import { ChatStreamParser } from "./chat-stream-parser";

const encoder = new TextEncoder();

describe("ChatStreamParser", () => {
  it("preserves frame order across UTF-8 and SSE chunk boundaries", () => {
    const parser = new ChatStreamParser();
    const source = [
      'data: {"type":"RUN_STARTED","turnId":"turn-1"}\n\n',
      'data: {"type":"TEXT_MESSAGE_CONTENT","textMessageContent":{"delta":"hé"}}\n\n',
    ].join("");
    const bytes = encoder.encode(source);
    const split = bytes.indexOf(0xc3) + 1;

    expect(parser.push(bytes.slice(0, split))).toEqual([
      { type: "RUN_STARTED", turnId: "turn-1" },
    ]);
    expect(parser.push(bytes.slice(split))).toEqual([
      {
        type: "TEXT_MESSAGE_CONTENT",
        textMessageContent: { delta: "hé" },
      },
    ]);
    expect(parser.finish()).toEqual([]);
  });

  it("flushes a final frame without a trailing SSE separator", () => {
    const parser = new ChatStreamParser();

    expect(
      parser.push(encoder.encode('data: {"type":"RUN_FINISHED"}')),
    ).toEqual([]);
    expect(parser.finish()).toEqual([{ type: "RUN_FINISHED" }]);
  });

  it("drops malformed and non-object payloads without dropping valid frames", () => {
    const parser = new ChatStreamParser();
    const frames = parser.push(
      encoder.encode(
        [
          "data: not-json",
          "",
          "data: []",
          "",
          'data: {"type":"RUN_STARTED","turnId":"turn-2"}',
          "",
          "",
        ].join("\n"),
      ),
    );

    expect(frames).toEqual([{ type: "RUN_STARTED", turnId: "turn-2" }]);
  });
});

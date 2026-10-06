import { describe, expect, it } from "vitest";
import {
  desktopFrame,
  desktopInput,
  desktopKey,
  desktopPoint,
  desktopUrl,
} from "./machine-desktop";
const id = "12345678-1234-4234-8234-123456789abc";
describe("desktop transport and input", () => {
  it("binds frames to their session and enforces byte caps", () => {
    const bytes = desktopInput(id, 9n, "click", { x: 10, y: 20 });
    new Uint8Array(bytes)[4] = 1;
    expect(desktopFrame(bytes, id)?.sequence).toBe(9n);
    expect(
      desktopFrame(bytes, "00000000-0000-0000-0000-000000000000"),
    ).toBeUndefined();
    expect(desktopFrame(new ArrayBuffer(6 * 1024 * 1024), id)).toBeUndefined();
    expect(() =>
      desktopInput(id, 1n, "type_text", { text: "a".repeat(20000) }),
    ).toThrow();
    expect(() => desktopInput("invalid", 1n, "click", {})).toThrow();
  });
  it("maps object-contain pixels and ignores letterbox clicks", () => {
    const rect = { left: 10, top: 20, width: 1000, height: 1000 };
    expect(desktopPoint(rect, 1280, 800, 510, 520)).toEqual({ x: 640, y: 400 });
    expect(desktopPoint(rect, 1280, 800, 510, 21)).toBeUndefined();
    expect(desktopPoint(rect, 1280, 800, 1010, 520)).toBeUndefined();
  });
  it("maps browser keys to the portable cua contract and chooses authenticated WS", () => {
    expect(desktopKey("ArrowLeft")).toBe("LEFT");
    expect(desktopKey("Escape")).toBe("ESC");
    expect(desktopKey("PageDown")).toBe("PAGEDOWN");
    expect(
      desktopUrl("node", "thread", "https://nyxid.example/api/v1/nodes/ws"),
    ).toBe(
      "wss://nyxid.example/api/v1/assistant/nyxagent/machines/node/desktop?conversation_id=thread",
    );
    expect(
      desktopUrl("node", "thread", "https://nyxid.example/api/v1/nodes/ws", "secure", "ctx-1"),
    ).toContain("context_id=ctx-1");
  });
});

it("validates JPEG dirty rectangles and preserves their required base sequence", () => {
  const session = "11111111-2222-3333-4444-555555555555";
  const packet = new Uint8Array(30 + 24 + 3);
  packet.set([78, 89, 88, 77, 1, 0]);
  const id = session.replaceAll("-", "");
  for (let n = 0; n < 16; n++)
    packet[6 + n] = Number.parseInt(id.slice(n * 2, n * 2 + 2), 16);
  const view = new DataView(packet.buffer);
  view.setBigUint64(22, 9n);
  packet.set([78, 89, 88, 68], 30);
  [1280, 800, 64, 128, 64, 64].forEach((n, i) => view.setUint16(34 + i * 2, n));
  view.setBigUint64(46, 8n);
  const frame = desktopFrame(packet.buffer, session)!;
  expect(frame.rectangle).toEqual({
    width: 1280,
    height: 800,
    x: 64,
    y: 128,
    patchWidth: 64,
    patchHeight: 64,
    base: 8n,
  });
  expect(frame.sequence).toBe(9n);
  view.setBigUint64(46, 0n);
  expect(desktopFrame(packet.buffer, session)).toBeUndefined();
  view.setBigUint64(46, 8n);
  view.setUint16(42, 1920);
  expect(desktopFrame(packet.buffer, session)).toBeUndefined();
});

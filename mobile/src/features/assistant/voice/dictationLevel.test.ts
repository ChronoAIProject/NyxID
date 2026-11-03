import assert from "node:assert/strict";
import test from "node:test";
import { formatElapsed, pcmLevel } from "./dictationLevel";

function packet(amplitude: number) {
  const view = new DataView(new ArrayBuffer(960));
  for (let i = 0; i < 480; i++)
    view.setInt16(
      i * 2,
      Math.round((i % 2 ? -1 : 1) * amplitude * 32767),
      true,
    );
  return view.buffer;
}

test("PCM level maps silence to zero and loud speech toward one", () => {
  assert.equal(pcmLevel(new ArrayBuffer(0)), 0);
  assert.equal(pcmLevel(packet(0)), 0);
  assert.equal(pcmLevel(packet(0.001)), 0);
  const quiet = pcmLevel(packet(0.01));
  const loud = pcmLevel(packet(0.3));
  assert.ok(quiet > 0 && quiet < loud, `${quiet} < ${loud}`);
  assert.equal(pcmLevel(packet(1)), 1);
});

test("elapsed time reads as minutes and padded seconds", () => {
  assert.equal(formatElapsed(0), "0:00");
  assert.equal(formatElapsed(7_900), "0:07");
  assert.equal(formatElapsed(125_000), "2:05");
  assert.equal(formatElapsed(-5), "0:00");
});

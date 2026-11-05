import assert from "node:assert/strict";
import test from "node:test";
import { formatElapsed, volumeLevel } from "./dictationLevel";

test("recognizer volume maps inaudible input to zero and full scale to one", () => {
  assert.equal(volumeLevel(-2), 0);
  assert.equal(volumeLevel(-0.5), 0);
  assert.equal(volumeLevel(0), 0);
  assert.equal(volumeLevel(5), 0.5);
  assert.equal(volumeLevel(10), 1);
  assert.equal(volumeLevel(12), 1);
  assert.equal(volumeLevel(Number.NaN), 0);
});

test("elapsed time reads as minutes and padded seconds", () => {
  assert.equal(formatElapsed(0), "0:00");
  assert.equal(formatElapsed(7_900), "0:07");
  assert.equal(formatElapsed(125_000), "2:05");
  assert.equal(formatElapsed(-5), "0:00");
});

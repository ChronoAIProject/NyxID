import assert from "node:assert/strict";
import test from "node:test";
import { scrollEdges } from "./scrollEdges";
test("fades reveal only content beyond the measured scroll edges", () => {
  assert.deepEqual(scrollEdges(0, 1000, 0), { start: false, end: false });
  assert.deepEqual(scrollEdges(500, 500, 0), { start: false, end: false });
  assert.deepEqual(scrollEdges(500, 900, 0), { start: false, end: true });
  assert.deepEqual(scrollEdges(500, 900, 200), { start: true, end: true });
  assert.deepEqual(scrollEdges(500, 900, 400), { start: true, end: false });
  assert.deepEqual(scrollEdges(500, 900, -40), { start: false, end: true });
  assert.deepEqual(scrollEdges(500, 900, 460), { start: true, end: false });
});
test("prepending, footer contraction and keyboard resize recalculate available edges", () => {
  assert.deepEqual(scrollEdges(500, 1000, 500), { start: true, end: false });
  assert.deepEqual(scrollEdges(500, 1300, 500), { start: true, end: true });
  assert.deepEqual(scrollEdges(500, 800, 500), { start: true, end: false });
  assert.deepEqual(scrollEdges(280, 800, 300), { start: true, end: true });
  assert.deepEqual(scrollEdges(800, 800, 300), { start: false, end: false });
});

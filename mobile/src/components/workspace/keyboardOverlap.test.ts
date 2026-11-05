import assert from "node:assert/strict";
import test from "node:test";
import { keyboardOverlap } from "./keyboardOverlap";

test("edge-to-edge content reserves only its keyboard overlap, below the header", () => {
  assert.equal(
    keyboardOverlap({ y: 92, height: 774 }, { screenY: 580, height: 286 }),
    286,
  );
  assert.equal(
    keyboardOverlap({ y: 132, height: 734 }, { screenY: 580, height: 286 }),
    286,
    "a taller header does not change the bottom overlap",
  );
});

test("a window already resized above the IME gets no second keyboard inset", () => {
  assert.equal(
    keyboardOverlap({ y: 92, height: 488 }, { screenY: 580, height: 286 }),
    0,
  );
  assert.equal(
    keyboardOverlap({ y: 92, height: 460 }, { screenY: 580, height: 286 }),
    0,
    "an existing safe-area gap is not added again",
  );
});

test("partial resize and gesture navigation reserve only the remaining overlap", () => {
  assert.equal(
    keyboardOverlap({ y: 92, height: 600 }, { screenY: 580, height: 286 }),
    112,
  );
  assert.equal(
    keyboardOverlap({ y: 92, height: 798 }, { screenY: 580, height: 310 }),
    310,
  );
});

test("layout changes are calculated from the current view and IME frames", () => {
  assert.equal(
    keyboardOverlap({ y: 60, height: 340 }, { screenY: 230, height: 170 }),
    170,
  );
  assert.equal(
    keyboardOverlap({ y: 250, height: 100 }, { screenY: 230, height: 170 }),
    100,
    "never reserve more than the available view height",
  );
});

test("hidden keyboard, unmounted layout and invalid measurements reserve no space", () => {
  const frame = { y: 92, height: 774 };
  assert.equal(keyboardOverlap(frame), 0);
  assert.equal(keyboardOverlap(frame, { screenY: 580, height: 0 }), 0);
  assert.equal(keyboardOverlap(frame, { screenY: NaN, height: 286 }), 0);
  assert.equal(keyboardOverlap(frame, { screenY: 580, height: Infinity }), 0);
  assert.equal(
    keyboardOverlap({ y: 0, height: 0 }, { screenY: 580, height: 286 }),
    0,
  );
});

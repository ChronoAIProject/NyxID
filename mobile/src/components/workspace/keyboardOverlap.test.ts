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

test("Android IME height excluding the navigation bar still lifts the whole viewport above the IME top", () => {
  assert.equal(
    keyboardOverlap({ y: 0, height: 914 }, { screenY: 611, height: 279 }, "android"),
    303,
    "bottom padding must include the 24pt below the reported IME frame",
  );
});

test("an iPad floating keyboard retains intersection-only handling", () => {
  assert.equal(
    keyboardOverlap({ y: 0, height: 800 }, { screenY: 200, height: 120 }, "ios"),
    120,
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

test("visible keyboard intersection clips partially offscreen frames and rejects oversized or above-window frames", async () => {
  const { visibleKeyboardFrame } = await import(
    "./keyboardOverlap"
  );
  assert.equal(
    visibleKeyboardFrame({ screenY: -20, height: 900 }, 852),
    undefined,
  );
  assert.equal(
    visibleKeyboardFrame({ screenY: 0, height: 1000 }, 852),
    undefined,
  );
  assert.equal(
    visibleKeyboardFrame({ screenY: 900, height: 300 }, 852),
    undefined,
  );
  const clipped = visibleKeyboardFrame({ screenY: 700, height: 300 }, 852);
  assert.deepEqual(clipped, { screenY: 700, height: 152 });
  assert.equal(keyboardOverlap({ y: 92, height: 760 }, clipped), 152);
  assert.equal(keyboardOverlap({ y: 0, height: 852 }, clipped), 152);
  assert.equal(
    keyboardOverlap({ y: 800, height: 52 }, { screenY: 500, height: 100 }),
    0,
  );
});

import assert from "node:assert/strict";
import test from "node:test";
import {
  sidebarInset,
  SLAB_MOTION_DURATION,
  SLAB_MOTION_EASING,
} from "./workspaceSlabTransitions";

test("slab transitions share one ~250ms ease-out on the slab's own curve", () => {
  assert.equal(SLAB_MOTION_DURATION, 250);
  assert.deepEqual([...SLAB_MOTION_EASING], [0.2, 0.8, 0.2, 1]);
});

test("the persistent sidebar's share of the window follows its shown progress", () => {
  assert.equal(sidebarInset(300, 1), 300);
  assert.equal(sidebarInset(300, 0), 0);
  assert.equal(sidebarInset(300, 0.5), 150);
  // Monotonic, so the slab's edge never jumps back while it moves.
  let last = -1;
  for (const shown of [0, 0.2, 0.4, 0.6, 0.8, 1]) {
    assert.ok(sidebarInset(300, shown) >= last);
    last = sidebarInset(300, shown);
  }
});

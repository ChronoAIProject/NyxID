import assert from "node:assert/strict";
import test from "node:test";
import {
  sidebarPushWidth,
  clampSidebarOffset,
  settleSidebarOpen,
  workspaceSidebarLayout,
} from "./workspacePushState";

test("compact push exposes 77% on phones and caps sidebar width in larger compact windows", () => {
  for (const width of [320, 393, 402, 430, 768, 1024, 1440]) {
    const exposed = sidebarPushWidth(width);
    assert.equal(exposed, Math.min(360, Math.round(width * 0.77)));
    assert.ok(width - exposed >= 44);
    assert.equal(clampSidebarOffset(-200, exposed), 0);
    assert.equal(clampSidebarOffset(9999, exposed), exposed);
  }
});
test("the actual window width chooses a persistent sidebar with at least 600pt for content", () => {
  for (const width of [320, 402, 600, 744, 834, 899]) {
    const layout = workspaceSidebarLayout(width);
    assert.equal(layout.persistent, false);
    assert.equal(layout.sidebarWidth, sidebarPushWidth(width));
  }
  for (const width of [900, 1024, 1194, 1366]) {
    const layout = workspaceSidebarLayout(width);
    assert.equal(layout.persistent, true);
    assert.equal(layout.sidebarWidth, 300);
    assert.ok(width - layout.sidebarWidth >= 600);
  }
  assert.deepEqual(workspaceSidebarLayout(-10), {
    persistent: false,
    sidebarWidth: 0,
  });
});
test("settlement uses45% travel and projected flick velocity in both directions", () => {
  const width = 310;
  assert.equal(settleSidebarOpen(138, 0, width), false);
  assert.equal(settleSidebarOpen(140, 0, width), true);
  assert.equal(settleSidebarOpen(40, 900, width), true);
  assert.equal(settleSidebarOpen(250, -900, width), false);
  assert.equal(settleSidebarOpen(0, -400, width), false);
  assert.equal(settleSidebarOpen(0, 1000, 0), false);
});

import assert from "node:assert/strict";
import test from "node:test";
import {
  sidebarPushWidth,
  clampSidebarOffset,
  settleSidebarOpen,
} from "./workspacePushState";

test("push preserves the whole content width while exposing77% at portrait and wide widths", () => {
  for (const width of [320, 393, 402, 430, 768, 1024, 1440]) {
    const exposed = sidebarPushWidth(width);
    assert.equal(exposed, Math.round(width * 0.77));
    assert.ok(width - exposed >= 44);
    assert.equal(clampSidebarOffset(-200, exposed), 0);
    assert.equal(clampSidebarOffset(9999, exposed), exposed);
  }
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

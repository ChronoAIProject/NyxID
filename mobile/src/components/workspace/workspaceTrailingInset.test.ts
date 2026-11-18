import assert from "node:assert/strict";
import test from "node:test";
import { ownTrailingPadding } from "./workspaceTrailingInset";

test("the rail inset animates on top of the container's own resolved trailing padding", () => {
  assert.equal(ownTrailingPadding({ padding: 20 }), 20);
  assert.equal(ownTrailingPadding([{ padding: 20 }, { paddingHorizontal: 0 }]), 0);
  assert.equal(ownTrailingPadding([{ padding: 20 }, [null, { paddingRight: 4 }]]), 4);
  assert.equal(ownTrailingPadding(undefined), 0);
});

test("rail mode: every vertical scroll indicator sits between the content and the rail's capsules", async () => {
  const {
    DUO_STATUS_COLUMN_CENTER_FROM_RIGHT,
    FLOATING_TRAILING_INSET: inset,
    QUICK_RAIL_TAB_SIZE,
  } = await import("./workspaceLayout");
  const {
    floatingScrollIndicatorProps,
    IOS_SCROLL_INDICATOR_EDGE_GAP,
    IOS_SCROLL_INDICATOR_WIDTH,
  } = await import("./workspaceTrailingInset");
  const width = 466; // folded Duo
  const capsule = width - (DUO_STATUS_COLUMN_CENTER_FROM_RIGHT + QUICK_RAIL_TAB_SIZE / 2);
  // Every scroller sits inside a rail-inset container (WorkspacePage narrows
  // its own scroller too); content keeps at least a 12pt margin inside it.
  const scrollerRight = width - inset;
  const contentRight = scrollerRight - 12;
  const props = floatingScrollIndicatorProps(inset, false);
  assert.equal(props.automaticallyAdjustsScrollIndicatorInsets, false);
  assert.equal("contentInset" in props, false, "content never sits under the rail");
  const right = scrollerRight - props.scrollIndicatorInsets!.right - IOS_SCROLL_INDICATOR_EDGE_GAP;
  const left = right - IOS_SCROLL_INDICATOR_WIDTH;
  assert.ok(left > contentRight, `clear of the content (${left} > ${contentRight})`);
  assert.ok(right < capsule, `clear of the capsules (${right} < ${capsule})`);
  // Without the inset it would touch the capsules (the bug this fixed).
  assert.ok(scrollerRight - IOS_SCROLL_INDICATOR_EDGE_GAP > capsule);
  assert.deepEqual(floatingScrollIndicatorProps(0, false), {}, "no rail, native placement");
  assert.deepEqual(floatingScrollIndicatorProps(0, true), {});
});

test("full screen: the indicator moves to the window edge and content keeps its natural top (no resting gap)", async () => {
  const { FLOATING_TRAILING_INSET: inset } = await import("./workspaceLayout");
  const { floatingScrollIndicatorProps } = await import("./workspaceTrailingInset");
  assert.deepEqual(floatingScrollIndicatorProps(inset, true), {
    scrollIndicatorInsets: { right: 0 },
    automaticallyAdjustsScrollIndicatorInsets: false,
  });
});

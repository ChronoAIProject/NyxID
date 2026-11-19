import assert from "node:assert/strict";
import test from "node:test";
import * as layout from "./workspaceLayout";
import {
  CORNER_HEADER_TOP_GAP,
  DUO_SENSOR_BOTTOM,
  DUO_STATUS_COLUMN_LEFT_FROM_RIGHT,
  DUO_STATUS_COLUMN_CENTER_FROM_RIGHT,
  FLOATING_TRAILING_INSET,
  workspaceBodyEdges,
  workspaceStatusCluster,
  workspaceHeaderBand,
  workspaceHeaderRightPadding,
  workspaceCornerSensor,
  workspacePanelContent,
  workspaceQuickRailFrame,
  workspaceQuickRailToggleFrame,
  workspaceRightPanelLayout,
  sidebarSectionHeaderMetaVisible,
} from "./workspaceLayout";
import { FLOATING_HEADER_MARGIN } from "./workspaceHeaderMetrics";

const FOLDED = { platform: "ios", width: 466, height: 678, rightInset: 83 };
const UNFOLDED = { platform: "ios", width: 951, height: 669, rightInset: 104 };
// Measured in duo-folded-en-notifications-default.png (points).
const CLUSTER_INK_LEFT = 397.67;
const CAMERA_INK_TOP = 29.0;
import {
  defaultChatLayout,
  transcriptColumnStyle,
} from "../../features/display/chatLayout";

test("corner sensor detection excludes ordinary iPhones, landscape notches and inset-free iPads", () => {
  for (const [width, height, rightInset, corner] of [
    [466, 678, 83, true], [678, 466, 104, true], [951, 669, 104, true],
    [393, 852, 0, false], [852, 393, 59, false], [874, 402, 104, false],
    [834, 1194, 0, false], [600, 700, 0, false], [1366, 1024, 0, false],
    [899, 700, 79, false],
  ] as const)
    assert.equal(workspaceCornerSensor({ platform: "ios", width, height, rightInset }), corner);
});
test("panel routes home groups and conversation details without duplicating home groups inline", () => {
  assert.equal(workspacePanelContent(true, false), "home");
  assert.equal(workspacePanelContent(false, true), "agent");
  assert.equal(workspacePanelContent(false, false), "loading");
});
test("beside a corner cluster bodies run to the right edge; landscape notches stay protected", () => {
  assert.deepEqual(workspaceBodyEdges(true), ["bottom", "left"]);
  assert.deepEqual(workspaceBodyEdges(false), ["bottom", "left", "right"]);
});
test("beside the Duo's status column the header row sits flush at the top; content keeping the rail inset starts right under it", () => {
  const folded = workspaceStatusCluster(FOLDED)!;
  assert.equal(CORNER_HEADER_TOP_GAP, 8);
  // Folded, rail shown or full screen alike (content keeps the rail inset):
  // row 8–68pt, flush at the top, and content starts right under it (68pt,
  // was 152pt): 84pt more for content, with the row 54.5pt higher.
  const band = workspaceHeaderBand(0, true, folded, 60, true);
  assert.deepEqual(band, { paddingTop: 8, paddingBottom: 0 });
  assert.ok(band.paddingTop + 60 < DUO_SENSOR_BOTTOM);
  // The camera dot (29–66pt) and clock are in the status column only; the
  // row's controls end left of it (next test), so they share the top band.
  assert.ok(band.paddingTop < CAMERA_INK_TOP);
  // Full-width bodies (sheets, and a folded body without the rail inset)
  // still start below the cluster.
  // A sheet's row sits right where its header begins (under the handle).
  const sheet = workspaceHeaderBand(54, true, folded);
  assert.equal(sheet.paddingTop, 54);
  assert.equal(sheet.paddingTop + 60 + sheet.paddingBottom, DUO_SENSOR_BOTTOM);
  assert.equal(
    workspaceHeaderBand(0, true, folded).paddingBottom,
    DUO_SENSOR_BOTTOM - 68,
  );
  // Unfolded landscape: row 8–68pt (was 10–70pt); its full-width bodies
  // start below the cluster (100pt), measured to 98.0pt.
  const unfolded = workspaceStatusCluster(UNFOLDED)!;
  assert.ok(unfolded.bottom >= 98);
  assert.deepEqual(workspaceHeaderBand(0, true, unfolded), {
    paddingTop: 8,
    paddingBottom: 32,
  });
  // Unmeasured corner postures: flush too, clearing the folded height.
  const foldedLandscape = { platform: "ios", width: 678, height: 466, rightInset: 104 };
  assert.equal(workspaceStatusCluster(foldedLandscape), undefined);
  assert.deepEqual(workspaceHeaderBand(0, true, undefined), {
    paddingTop: 8,
    paddingBottom: DUO_SENSOR_BOTTOM - 68,
  });
  // Ordinary iPhones are unchanged: the row sits at their real top inset.
  for (const metrics of [
    { platform: "ios", width: 402, height: 874, rightInset: 0 },
    { platform: "ios", width: 852, height: 393, rightInset: 59 },
    { platform: "ios", width: 1366, height: 1024, rightInset: 0 },
  ]) {
    assert.equal(workspaceStatusCluster(metrics), undefined);
    assert.equal(workspaceCornerSensor(metrics), false);
    assert.deepEqual(workspaceHeaderBand(62, false, undefined), {
      paddingTop: 62,
      paddingBottom: 0,
    });
    assert.deepEqual(workspaceHeaderBand(62, false, undefined, 60, true), {
      paddingTop: 62,
      paddingBottom: 0,
    });
  }
  // Never above the safe-area inset.
  assert.equal(workspaceHeaderBand(80, true, folded).paddingTop, 80);
});
test("trailing header controls end left of the cluster column in every Duo posture", () => {
  for (const [metrics, margin] of [
    [FOLDED, FLOATING_HEADER_MARGIN],
    [UNFOLDED, 8],
  ] as const) {
    const trailingEdge =
      metrics.width - workspaceHeaderRightPadding(metrics.rightInset, margin);
    assert.ok(trailingEdge <= metrics.width - metrics.rightInset);
  }
  assert.ok(
    FOLDED.width - workspaceHeaderRightPadding(83, FLOATING_HEADER_MARGIN) <
      CLUSTER_INK_LEFT,
  );
  // The flush row shares the top band with the status column: every
  // posture's trailing controls end left of the column's measured ink.
  for (const [metrics, margin] of [
    [FOLDED, FLOATING_HEADER_MARGIN],
    [UNFOLDED, 8],
  ] as const)
    assert.ok(
      metrics.width - workspaceHeaderRightPadding(metrics.rightInset, margin) <=
        metrics.width - DUO_STATUS_COLUMN_LEFT_FROM_RIGHT,
    );
  assert.ok(FOLDED.width - DUO_STATUS_COLUMN_LEFT_FROM_RIGHT <= CLUSTER_INK_LEFT);
});
test("panel preserves the live transcript column including its native padding", () => {
  for (const column of [360, defaultChatLayout.columnWidth, 1200]) {
    const threshold = 300 + transcriptColumnStyle(column).maxWidth + 280;
    assert.equal(
      workspaceRightPanelLayout(threshold - 1, column).enabled,
      false,
    );
    const layout = workspaceRightPanelLayout(threshold, column);
    assert.equal(layout.enabled, true);
    assert.equal(layout.chatWidth, transcriptColumnStyle(column).maxWidth);
    assert.equal(
      workspaceRightPanelLayout(threshold, column, 1).enabled,
      false,
    );
  }
  assert.equal(workspaceRightPanelLayout(951).enabled, false);
  assert.equal(workspaceRightPanelLayout(1366).enabled, true);
});
test("only corner insets are reclaimed below the sensor band", () => {
  assert.equal(workspaceHeaderRightPadding(0), 8);
  assert.equal(workspaceHeaderRightPadding(104), 112);
  assert.deepEqual(
    workspaceBodyEdges(
      workspaceCornerSensor({ platform: "ios", width: 678, height: 466, rightInset: 104 }),
    ),
    ["bottom", "left"],
  );
  assert.deepEqual(
    workspaceBodyEdges(
      workspaceCornerSensor({ platform: "ios", width: 852, height: 393, rightInset: 59 }),
    ),
    ["bottom", "left", "right"],
  );
});
test("measured metadata yields before a single-line heading at large type", () => {
  assert.equal(
    sidebarSectionHeaderMetaVisible(232, 110, 70, 16, 88, 26),
    false,
  );
  assert.equal(sidebarSectionHeaderMetaVisible(360, 110, 70, 16, 88, 26), true);
  assert.equal(
    sidebarSectionHeaderMetaVisible(360, 220, 140, 16, 88, 26),
    false,
  );
});
test("the quick rail is on every corner-sensor window (folded and unfolded Duo) and starts below the cluster", () => {
  assert.deepEqual(Object.keys(layout).sort(), [
    "CENTERED_SHEET_MIN_WIDTH",
    "CORNER_HEADER_TOP_GAP",
    "DUO_SENSOR_BOTTOM",
    "DUO_STATUS_COLUMN_CENTER_FROM_RIGHT",
    "DUO_STATUS_COLUMN_LEFT_FROM_RIGHT",
    "FLOATING_TRAILING_INSET",
    "QUICK_RAIL_BUTTON_GAP",
    "QUICK_RAIL_GAP",
    "QUICK_RAIL_MAX_STATUS_OFFSET",
    "QUICK_RAIL_SECTION_GAP",
    "QUICK_RAIL_TAB_SIZE",
    "QUICK_RAIL_WIDTH",
    "RIGHT_PANEL_WIDTH",
    "quickRailCapsuleHeight",
    "sidebarSectionHeaderMetaVisible",
    "workspaceBodyEdges",
    "workspaceCornerSensor",
    "workspaceHeaderBand",
    "workspaceHeaderRightPadding",
    "workspacePanelContent",
    "workspaceQuickRailAvailable",
    "workspaceQuickRailFrame",
    "workspaceQuickRailShortcutsViewport",
    "workspaceQuickRailToggleFrame",
    "workspaceRightPanelLayout",
    "workspaceSheetRailReserve",
    "workspaceStatusCluster",
  ]);
  // Bottom sheets end 8pt before the rail's column in either Duo posture
  // (rail 20pt from the edge, 56pt wide).
  for (const [width, height, rightInset, reserve] of [
    [466, 678, 83, 84], // folded Duo
    [678, 466, 104, 84], // folded Duo, landscape
    [951, 669, 104, 84], // unfolded Duo: bottom sheet too
    [402, 874, 0, 0], // ordinary iPhone
    [874, 402, 62, 0], // landscape iPhone notch
  ] as const)
    assert.equal(
      layout.workspaceSheetRailReserve({ platform: "ios", width, height, rightInset }),
      reserve,
      `${width}x${height}`,
    );
  for (const [width, height, rightInset, rail] of [
    [466, 678, 83, true], // folded Duo
    [678, 466, 104, true], // folded Duo, landscape
    [402, 874, 0, false], // ordinary iPhone
    [874, 402, 62, false], // landscape iPhone notch
    [951, 669, 104, true], // unfolded Duo, beside the persistent sidebar
    [1366, 1024, 0, false], // iPad: no corner cluster
    [1024, 768, 0, false],
    [834, 1194, 0, false],
  ] as const)
    assert.equal(
      layout.workspaceQuickRailAvailable({ platform: "ios", width, height, rightInset }),
      rail,
      `${width}x${height}`,
    );
  // A 44pt target column that fits the 83pt folded strip with clearance.
  assert.ok(layout.QUICK_RAIL_WIDTH >= 44 && layout.QUICK_RAIL_WIDTH + 12 <= 83);
  assert.equal(workspaceQuickRailFrame(0, 34).top, DUO_SENSOR_BOTTOM + 8);
  assert.equal(workspaceQuickRailFrame(200, 34).top, 196, "system growth respects the movement cap");
});

test("the rail's capsules center on the measured status column", () => {
  const column = FOLDED.width - DUO_STATUS_COLUMN_CENTER_FROM_RIGHT;
  // Clock 401.67–434.00pt and Wi‑Fi 397.67–438.67pt both center on 418pt.
  assert.ok(Math.abs(column - 418) <= 0.5);
  for (const frame of [workspaceQuickRailFrame(0, 34), workspaceQuickRailToggleFrame(0)]) {
    const center = FOLDED.width - frame.right - frame.width / 2;
    assert.ok(Math.abs(center - column) <= 0.5, `center ${center}`);
    // Within the window, and clear of the body's 83pt-inset edge.
    assert.ok(frame.right >= 0);
    assert.ok(FOLDED.width - frame.right - frame.width >= FOLDED.width - FOLDED.rightInset);
  }
});

test("the restore capsule is fully on screen: inside the window, inside the status column, below the cluster", () => {
  const toggle = workspaceQuickRailToggleFrame(0);
  const left = FOLDED.width - toggle.right - toggle.width;
  const right = FOLDED.width - toggle.right;
  // Inside the window.
  assert.ok(left >= 0 && right <= FOLDED.width, `x ${left}–${right}`);
  assert.ok(toggle.top >= 0 && toggle.bottom <= FOLDED.height);
  // Inside the status column: the 83pt right strip, centered on its 418pt
  // column like the clock and Wi‑Fi, with a margin from the window edge.
  assert.ok(left >= FOLDED.width - FOLDED.rightInset, `left ${left}`);
  assert.ok(FOLDED.width - right >= 20, "clear of the window edge");
  assert.ok(
    Math.abs((left + right) / 2 - (FOLDED.width - DUO_STATUS_COLUMN_CENTER_FROM_RIGHT)) <= 0.5,
  );
  // Idle keeps the original position; only activity moves it down.
  assert.deepEqual([left, toggle.top, right, toggle.bottom], [396, 160, 440, 204]);
  assert.equal(workspaceQuickRailToggleFrame(0, undefined, 20).top, 180);
  assert.equal(workspaceQuickRailToggleFrame(0, undefined, 36).top, 196);
  assert.equal(workspaceQuickRailToggleFrame(0, undefined, 100).top, 196, "never below the composer-aligned maximum");
});

test("Full screen and Show quick actions are one capsule at one spot for every top inset", () => {
  for (const topInset of [0, 20, 47, 59, 100, 170]) {
    const rail = workspaceQuickRailFrame(topInset, 34);
    const toggle = workspaceQuickRailToggleFrame(topInset);
    // The toggle is the column's first capsule.
    const fullScreenCenter = rail.top + 44 / 2;
    assert.ok(Math.abs(fullScreenCenter - toggle.centerY) <= 0.5, `top ${topInset}`);
    assert.equal(toggle.right, rail.right + (rail.width - toggle.width) / 2);
    assert.ok(toggle.top >= DUO_SENSOR_BOTTOM, "below the sensor cluster");
    assert.ok(toggle.width >= 44 && toggle.height >= 44);
    // The grouped shortcuts capsule follows a fixed spacing-token gap (12–16pt).
    assert.ok(layout.QUICK_RAIL_SECTION_GAP >= 12 && layout.QUICK_RAIL_SECTION_GAP <= 16);
  }
});

test("content keeps clear of the rail's capsules while backgrounds run underneath", () => {
  assert.equal(FLOATING_TRAILING_INSET, 66);
  const railLeft =
    FOLDED.width - workspaceQuickRailToggleFrame(0).right - workspaceQuickRailToggleFrame(0).width;
  assert.equal(railLeft, 396);
  // Content edge 12pt from the screen; content a further 66pt in, so it ends
  // at least 8pt left of the glass capsules, and left of the status column
  // under a flush header.
  assert.ok(FOLDED.width - 12 - FLOATING_TRAILING_INSET <= railLeft - 8);
  assert.ok(
    FOLDED.width - 12 - FLOATING_TRAILING_INSET <
      FOLDED.width - DUO_STATUS_COLUMN_LEFT_FROM_RIGHT,
  );
  // One grouped capsule: four 44pt targets 4pt apart; the column ends far
  // above the composer (idle toggle 160–204, grouped capsule 216–404pt).
  assert.equal(layout.QUICK_RAIL_BUTTON_GAP, 4);
  const bottom =
    workspaceQuickRailToggleFrame(0).bottom +
    layout.QUICK_RAIL_SECTION_GAP +
    4 * 44 +
    3 * layout.QUICK_RAIL_BUTTON_GAP;
  assert.equal(bottom, 404);
  assert.ok(bottom < 678 - 34 - 44);
});

test("unfolded Duo (951×669): the same rail under its measured cluster, the folded header's controls, content clear of the rail with the sidebar open or closed", () => {
  const cluster = workspaceStatusCluster(UNFOLDED)!;
  assert.equal(cluster.bottom, 100, "clock 34.0–45.7pt, Wi‑Fi and dots to 98.0pt");
  assert.equal(layout.workspaceQuickRailAvailable(UNFOLDED), true);
  // The idle rail column: centered on the status column
  // (48pt from the right edge, like the clock and Wi‑Fi), inside the 104pt
  // strip and the window.
  const column = UNFOLDED.width - DUO_STATUS_COLUMN_CENTER_FROM_RIGHT;
  const rail = workspaceQuickRailFrame(0, 20, cluster);
  assert.deepEqual(rail, { top: 108, bottom: 24, right: 20, width: 56 });
  const toggle = workspaceQuickRailToggleFrame(0, cluster);
  const toggleLeft = UNFOLDED.width - toggle.right - toggle.width;
  const toggleRight = UNFOLDED.width - toggle.right;
  assert.deepEqual([toggleLeft, toggle.top, toggleRight, toggle.bottom], [881, 108, 925, 152]);
  assert.ok(Math.abs((toggleLeft + toggleRight) / 2 - column) <= 0.5);
  assert.ok(toggleLeft >= UNFOLDED.width - UNFOLDED.rightInset, "inside the status strip");
  assert.ok(toggle.top >= 98 + 8, "below the clock and Wi‑Fi");
  // The grouped capsule follows (four 44pt targets 4pt apart) and ends far
  // above the composer.
  const groupTop = toggle.bottom + layout.QUICK_RAIL_SECTION_GAP;
  const groupBottom = groupTop + 4 * 44 + 3 * layout.QUICK_RAIL_BUTTON_GAP;
  assert.deepEqual([groupTop, groupBottom], [164, 352]);
  assert.ok(groupBottom < UNFOLDED.height - 34 - 44);
  // The same toggle spot in rail mode and full screen (one frame).
  // Header: flush 8–68pt like folded; content keeping the rail inset starts
  // right under the row. Trailing controls (only ⋯ beside the rail) end left
  // of the status column and of the rail's capsules.
  assert.deepEqual(workspaceHeaderBand(0, true, cluster, 60, true), {
    paddingTop: 8,
    paddingBottom: 0,
  });
  // The user's unfolded Duo (2026-11-17 capture): ⋯ glass 812–854pt, i.e. a
  // right inset of about 84pt; the fixture's 104pt is the conservative case.
  for (const [rightInset, trailing] of [
    [UNFOLDED.rightInset, 835],
    [84, 855],
  ] as const) {
    const headerTrailing =
      UNFOLDED.width - workspaceHeaderRightPadding(rightInset, FLOATING_HEADER_MARGIN);
    assert.equal(headerTrailing, trailing);
    assert.ok(headerTrailing <= UNFOLDED.width - DUO_STATUS_COLUMN_LEFT_FROM_RIGHT);
    assert.ok(headerTrailing < toggleLeft);
    // The rail never depends on the inset: it is anchored to the window edge.
    assert.ok(toggleLeft >= UNFOLDED.width - rightInset + 3);
  }
  // Rail mode: content ends 12pt + the 66pt inset from the right edge, at
  // least 8pt left of the capsules, whether the 300pt persistent sidebar is
  // open (slab 300–951pt) or closed (0–951pt); the sidebar sits at the
  // opposite edge and never reaches the rail.
  const contentRight = UNFOLDED.width - 12 - FLOATING_TRAILING_INSET;
  assert.equal(contentRight, 873);
  assert.ok(contentRight <= toggleLeft - 8);
  for (const slabLeft of [0, 300]) {
    assert.ok(slabLeft + 12 < contentRight, `slab from ${slabLeft}`);
    assert.ok(slabLeft < toggleLeft, "the sidebar never overlaps the rail");
  }
  // Full screen: content runs to the 12pt margin (+66pt width).
  assert.equal(UNFOLDED.width - 12 - contentRight, FLOATING_TRAILING_INSET);
});

test("the Duo's chats: in rail mode the column is the toggle, the shortcuts, then the bar's three controls; full screen brings the chat's own bar back", () => {
  for (const [metrics, toggleBottom, shortcutsBottom, headerBottom] of [
    [FOLDED, 240, 488, 640],
    [UNFOLDED, 188, 436, 588],
  ] as const) {
    const cluster = workspaceStatusCluster(metrics);
    assert.equal(workspaceQuickRailToggleFrame(0, cluster, 36).bottom, toggleBottom);
    // 12pt between capsules; five 44pt shortcuts and three header controls,
    // each 4pt apart. The column fits inside the window, beside the composer
    // (which keeps the rail inset).
    const gap = layout.QUICK_RAIL_SECTION_GAP;
    const capsule = (n: number) => n * 44 + (n - 1) * layout.QUICK_RAIL_BUTTON_GAP;
    assert.equal(toggleBottom + gap + capsule(5), shortcutsBottom);
    assert.equal(shortcutsBottom + gap + capsule(3), headerBottom);
    assert.ok(headerBottom < metrics.height, `${metrics.width}: ${headerBottom}`);
    if (metrics === FOLDED) {
      const rail = workspaceQuickRailFrame(0, 34, cluster, 36);
      assert.equal(headerBottom, metrics.height - 34 - 4, "rail and composer share the 640pt baseline");
      assert.equal(headerBottom, metrics.height - rail.bottom);
      assert.equal(
        layout.workspaceQuickRailShortcutsViewport(5, metrics.height - rail.bottom - rail.top, true).scrolls,
        false,
        "all default shortcuts fit at the maximum offset",
      );
    }
    // Full screen: the chat's own bar returns in the flush row (8–68pt),
    // its trailing controls left of the column, content right under it.
    assert.deepEqual(workspaceHeaderBand(0, true, cluster, 60, true), {
      paddingTop: 8,
      paddingBottom: 0,
    });
    // Vertical space gained on a chat in rail mode: the old 8–68pt bar.
    assert.equal(CORNER_HEADER_TOP_GAP + 60, 68);
  }
});

test("the shortcuts capsule hugs its quick-access entries, or takes the room left in the column and scrolls", () => {
  // 44pt targets 4pt apart.
  assert.equal(layout.quickRailCapsuleHeight(0), 0);
  assert.equal(layout.quickRailCapsuleHeight(1), 44);
  assert.equal(layout.quickRailCapsuleHeight(5), 5 * 44 + 4 * 4);
  // The folded Duo's column: 196pt to the composer's 640pt bottom = 444pt.
  // The toggle (44pt) and a 12pt gap come first;
  // on a chat in rail mode the ☰ / agent / ⋯ capsule (140pt) and another
  // 12pt gap come after, so 236pt are left; elsewhere 388pt.
  const column = 444;
  for (const [count, header, height, scrolls] of [
    [4, true, 188, false], // four shortcuts still hug their contents
    [5, true, 236, false], // the default five fit above the aligned chat controls
    [6, true, 236, true], // 284pt of entries: scroll in 236pt
    [12, true, 236, true],
    [8, false, 380, false], // no chat controls: eight entries still hug
    [9, false, 388, true],
    [10, false, 388, true],
    [12, false, 388, true],
  ] as const) {
    const viewport = layout.workspaceQuickRailShortcutsViewport(count, column, header);
    assert.equal(viewport.content, layout.quickRailCapsuleHeight(count));
    assert.deepEqual(
      [viewport.height, viewport.scrolls],
      [height, scrolls],
      `${count} entries${header ? " beside a chat's capsule" : ""}`,
    );
  }
  // Unmeasured: hug. Empty: nothing. A tiny column still shows one target.
  assert.deepEqual(layout.workspaceQuickRailShortcutsViewport(12, undefined, true), {
    content: 572,
    height: 572,
    scrolls: false,
  });
  assert.equal(layout.workspaceQuickRailShortcutsViewport(0, column, true).height, 0);
  assert.deepEqual(layout.workspaceQuickRailShortcutsViewport(4, 100, true), {
    content: 188,
    height: 44,
    scrolls: true,
  });
});

test('P10 only iOS corner windows get sensor headers, quick rail and sheet reserve', () => {
  for (const platform of ['android', 'web', 'windows', 'macos']) {
    for(const metrics of [FOLDED, UNFOLDED, { platform: "ios",width:700,height:850,rightInset:100}]) {
      const window={...metrics,platform};
      assert.equal(workspaceCornerSensor(window),false,`${platform} must retain ordinary headers`);
      assert.equal(layout.workspaceQuickRailAvailable(window),false);
      assert.equal(layout.workspaceSheetRailReserve(window),0);
      assert.equal(workspaceStatusCluster(window),undefined);
    }
  }
});

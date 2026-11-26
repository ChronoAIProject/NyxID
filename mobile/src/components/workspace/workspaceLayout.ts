import { workspaceSidebarLayout } from "./workspacePushState";
import { WORKSPACE_HEADER_ROW_HEIGHT } from "./workspaceHeaderMetrics";
import { spacing } from "../../theme/designTokens";
import { FLOATING_CONTROL_SIZE } from "../floatingControlTokens";
import {
  defaultChatLayout,
  transcriptColumnStyle,
} from "../../features/display/chatLayout";

// Duo display=1 is 1398×2034 pixels at 3× = 466×678pt folded (678×466
// rotated); display=3 is 669×951pt (951×669 rotated). Measured in
// nyx-duo-folded-before-fixed-wake.png: the last status-cluster pixel is
// y=453, or 151pt. Round outward to 152pt.
export const DUO_SENSOR_BOTTOM = 152;
// The status cluster's column, measured in duo-folded-en-notifications-default.png
// (default text size, 1398×2034 px at 3×): every band of ink centers on
// x = 418.0 ± 0.2pt of the 466pt display (camera 399.33–437.00pt, clock
// 401.67–434.00pt, Wi‑Fi 397.67–438.67pt, signal dots 406.33–430.00pt). That
// is 48pt from the right edge; the rail's capsules center there so the
// clock, Wi‑Fi and rail read as one column. The workspace is portrait-only on
// phones (only the live desktop rotates), so the portrait measurement holds.
export const DUO_STATUS_COLUMN_CENTER_FROM_RIGHT = 48;
// Same capture: the clock's ink spans y = 86.67–98.33pt (camera dot
// 29.00–66.33pt, Wi‑Fi 109.67–141.00pt). The camera and clock occupy only this
// right column, so header rows beside it sit flush at the top (see
// workspaceHeaderBand), left of the column.
/**
 * Status clusters measured per posture (window in points). Folded portrait:
 * see above (cluster to 152pt). Unfolded landscape (951×669, user-device
 * capture 2026-11-16): clock 34.0–45.7pt, Wi‑Fi and signal dots to 98.0pt,
 * column centered 48pt from the right edge.
 */
const DUO_STATUS_CLUSTERS = [
  { width: 466, height: 678, bottom: DUO_SENSOR_BOTTOM },
  { width: 951, height: 669, bottom: 100 },
] as const;
/**
 * The status column's left edge, measured from the right edge of the window:
 * its widest ink (Wi‑Fi, 397.67pt of 466) is 68.33pt from the right edge.
 * Header controls and readable content stay left of it.
 */
export const DUO_STATUS_COLUMN_LEFT_FROM_RIGHT = 69;
/** Beside a corner status column the header row starts at least this far from the window's top. */
export const CORNER_HEADER_TOP_GAP = spacing.sm;
// Measured folded Duo right inset: 83pt; ordinary landscape iPhone: 59pt.
// 80pt distinguishes the corner status strip, together with its squat aspect ratio.
const CORNER_SENSOR_MIN_RIGHT_INSET = 80;
export const RIGHT_PANEL_WIDTH = 280; // DESIGN.md opt-in panel slot, including padding.
// A column of 44pt targets; it fits the 83pt folded strip with edge clearance.
export const QUICK_RAIL_WIDTH = 56;
// The rail (its full-screen toggle capsule) starts this far below the sensor
// cluster.
export const QUICK_RAIL_GAP = spacing.sm;
// Between the toggle capsule and the grouped shortcuts capsule.
export const QUICK_RAIL_SECTION_GAP = spacing.lg;
// Between the 44pt shortcut targets inside the one grouped glass capsule.
export const QUICK_RAIL_BUTTON_GAP = spacing.xs;
// The capsules' width and each control's target: the full-screen toggle's own
// capsule (Full screen / Show quick actions) and each grouped shortcut.
export const QUICK_RAIL_TAB_SIZE = FLOATING_CONTROL_SIZE;
// Narrower sheets span the window, so a corner cluster can cover their top-right.
export const CENTERED_SHEET_MIN_WIDTH = 768;

export type WorkspaceWindowMetrics = {
  platform: string;
  width: number;
  height: number;
  rightInset: number;
};

/** A corner status strip, unlike the full-height notch inset on ordinary phones. */
export function workspaceCornerSensor(metrics: WorkspaceWindowMetrics) {
  return (
    metrics.platform === "ios" &&
    metrics.rightInset >= CORNER_SENSOR_MIN_RIGHT_INSET &&
    Math.max(metrics.width, metrics.height) /
      Math.min(metrics.width, metrics.height) <
      1.6
  );
}

/** Every window with a corner status column (the folded and the unfolded Duo,
 * narrow or beside the persistent sidebar) has the quick rail; ordinary
 * phones, landscape notches and iPads never do. */
export function workspaceQuickRailAvailable(metrics: WorkspaceWindowMetrics) {
  return workspaceCornerSensor(metrics);
}

/**
 * How much of a window's trailing edge a bottom sheet leaves to the quick
 * rail: on a corner-sensor window (either Duo posture) the sheet ends 8pt
 * before the rail's column, so the column stays in view beside it, never
 * under the sheet. Windows without the rail reserve nothing.
 */
export function workspaceSheetRailReserve(metrics: WorkspaceWindowMetrics) {
  if (!workspaceQuickRailAvailable(metrics)) return 0;
  const frame = workspaceQuickRailFrame(0, 0);
  return frame.right + frame.width + spacing.sm;
}

// Move only with capture activity or a system status-column expansion.
// Cap the extra offset at 36pt so the default folded chat rail (444pt tall)
// starts at 196pt and
// ends at the composer's 640pt baseline (678pt window − 34pt safe area −
// 4pt page padding). All five shortcuts fit without scrolling.
export const QUICK_RAIL_MAX_STATUS_OFFSET = 36;

/** Idle: just below the cluster. Active: move immediately, up to the cap. */
function quickRailTop(
  topInset: number,
  cluster?: WorkspaceStatusCluster,
  statusOffset = 0,
) {
  const idleTop = (cluster?.bottom ?? DUO_SENSOR_BOTTOM) + QUICK_RAIL_GAP;
  return Math.min(
    idleTop + QUICK_RAIL_MAX_STATUS_OFFSET,
    Math.max(idleTop + Math.max(0, statusOffset), topInset + QUICK_RAIL_GAP),
  );
}

/**
 * The quick rail's column: centered on the status column, from just below the
 * cluster, bounded by the body's bottom edge. Its two glass capsules (the
 * full-screen toggle, then the grouped shortcuts) hug their contents at the
 * top; the rest of the column is empty and passes touches through. The 56pt
 * width starts 7pt right of the body's 83pt-inset edge.
 */
export function workspaceQuickRailFrame(
  topInset: number,
  bottomInset: number,
  cluster?: WorkspaceStatusCluster,
  statusOffset = 0,
) {
  return {
    top: quickRailTop(topInset, cluster, statusOffset),
    // ChatSurface keeps 4pt below the composer inside the bottom safe area.
    bottom: Math.max(0, bottomInset) + spacing.xs,
    right: DUO_STATUS_COLUMN_CENTER_FROM_RIGHT - QUICK_RAIL_WIDTH / 2,
    width: QUICK_RAIL_WIDTH,
  };
}

/**
 * The full-screen toggle's own 44pt glass capsule, the same control in both
 * states so toggling never moves it under the finger: Full screen in rail
 * mode, Show quick actions (restore) in full screen. It centers on the status
 * column, directly under the cluster at the rail's top.
 */
export function workspaceQuickRailToggleFrame(
  topInset: number,
  cluster?: WorkspaceStatusCluster,
  statusOffset = 0,
) {
  const top = quickRailTop(topInset, cluster, statusOffset);
  return {
    top,
    right: DUO_STATUS_COLUMN_CENTER_FROM_RIGHT - QUICK_RAIL_TAB_SIZE / 2,
    width: QUICK_RAIL_TAB_SIZE,
    height: QUICK_RAIL_TAB_SIZE,
    bottom: top + QUICK_RAIL_TAB_SIZE,
    centerY: top + QUICK_RAIL_TAB_SIZE / 2,
  };
}

/** A rail capsule's height for `count` 44pt targets 4pt apart. */
export function quickRailCapsuleHeight(count: number) {
  return count > 0
    ? count * QUICK_RAIL_TAB_SIZE + (count - 1) * QUICK_RAIL_BUTTON_GAP
    : 0;
}

/** A chat's ☰ (or Back) / agent / ⋯ capsule below the shortcuts in rail mode. */
const RAIL_HEADER_CONTROL_COUNT = 3;

/**
 * The grouped shortcuts capsule for `count` quick-access entries in a column
 * `columnHeight` tall (the rail's column: below the cluster, down to the
 * body's bottom edge, which the composer's bottom shares). It hugs its items
 * when they fit between the toggle and, on a chat, the ☰ / agent / ⋯ capsule
 * (each 12pt apart); otherwise it takes the room left (at least one target)
 * and its items scroll inside it. Before the column is measured it hugs.
 */
export function workspaceQuickRailShortcutsViewport(
  count: number,
  columnHeight: number | undefined,
  headerInColumn: boolean,
) {
  const content = quickRailCapsuleHeight(count);
  if (columnHeight === undefined)
    return { content, height: content, scrolls: false };
  const available =
    columnHeight -
    QUICK_RAIL_TAB_SIZE -
    QUICK_RAIL_SECTION_GAP -
    (headerInColumn
      ? QUICK_RAIL_SECTION_GAP + quickRailCapsuleHeight(RAIL_HEADER_CONTROL_COUNT)
      : 0);
  const height = Math.max(
    Math.min(content, available),
    Math.min(content, QUICK_RAIL_TAB_SIZE),
  );
  return { content, height, scrolls: content > height };
}

/**
 * On the folded Duo the shared page containers (WorkspacePage,
 * ScreenContainer) add this to their trailing padding so nothing readable or
 * tappable sits under the floating rail's glass capsules (the full-screen
 * toggle and the grouped shortcuts); page backgrounds run underneath. 70pt
 * (a capsule's left edge: column centre 48 + half its 44pt width) + an 8pt
 * gap − the 12pt margin content keeps inside its container. Rail mode only:
 * full screen animates it to 0 so content runs to the 12pt margin under the
 * floating restore capsule. It also keeps rail-mode content clear of the
 * status column under a flush header.
 */
export const FLOATING_TRAILING_INSET =
  DUO_STATUS_COLUMN_CENTER_FROM_RIGHT + QUICK_RAIL_TAB_SIZE / 2 + spacing.sm - 12;

export type WorkspaceStatusCluster = { bottom: number };

/**
 * The corner status cluster where it has been measured for this posture;
 * other corner postures keep their header at the safe-area inset and clear
 * the folded cluster's height.
 */
export function workspaceStatusCluster(
  metrics: WorkspaceWindowMetrics,
): WorkspaceStatusCluster | undefined {
  if (!workspaceCornerSensor(metrics)) return undefined;
  return DUO_STATUS_CLUSTERS.find(
    (cluster) =>
      Math.abs(metrics.width - cluster.width) <= 1 &&
      Math.abs(metrics.height - cluster.height) <= 1,
  );
}

/**
 * A header's band. On ordinary iPhones the row sits at the top safe-area
 * inset, under the status bar. Beside a corner status column (the Duo) the
 * camera and clock occupy only that column, so the row sits flush at the top
 * (8pt from the window's edge, or directly where a sheet's header begins),
 * with its trailing controls left of the column
 * (`workspaceHeaderRightPadding`). Content that keeps the rail inset (the
 * folded Duo's pages) starts right under the row; content that runs to the
 * right edge (sheets, the unfolded posture) starts below the cluster.
 * `headerTop` is the window y where the header begins.
 */
export function workspaceHeaderBand(
  headerTop: number,
  cornerSensor: boolean,
  cluster?: WorkspaceStatusCluster,
  rowHeight = WORKSPACE_HEADER_ROW_HEIGHT,
  columnClear = false,
) {
  // Flush: where the header begins (a sheet's handle or padding), but at
  // least the small gap from the window's top edge.
  const rowTop = cornerSensor
    ? Math.max(headerTop, CORNER_HEADER_TOP_GAP)
    : headerTop;
  const rowBottom = rowTop + rowHeight;
  const contentTop =
    cornerSensor && !columnClear
      ? Math.max(rowBottom, cluster?.bottom ?? DUO_SENSOR_BOTTOM)
      : rowBottom;
  return { paddingTop: rowTop, paddingBottom: contentTop - rowBottom };
}

/** Preserve the configured transcript column and both of its native margins. */
export function workspaceRightPanelLayout(
  width: number,
  columnWidth = defaultChatLayout.columnWidth,
  horizontalInsets = 0,
) {
  const sidebar = workspaceSidebarLayout(width);
  const available =
    width - (sidebar.persistent ? sidebar.sidebarWidth : 0) - horizontalInsets;
  const readableWidth = transcriptColumnStyle(columnWidth).maxWidth;
  const enabled =
    sidebar.persistent && available >= readableWidth + RIGHT_PANEL_WIDTH;
  return {
    enabled,
    sidebarWidth: sidebar.persistent ? sidebar.sidebarWidth : 0,
    chatWidth: enabled ? available - RIGHT_PANEL_WIDTH : available,
    panelWidth: enabled ? RIGHT_PANEL_WIDTH : 0,
    readableWidth,
  };
}

export function workspaceHeaderRightPadding(rightInset: number, base = 8) {
  return base + Math.max(0, rightInset || 0);
}

/** All widths come from the rendered row, including scaled text and controls. */
export function sidebarSectionHeaderMetaVisible(
  rowWidth: number,
  labelWidth: number,
  metaWidth: number,
  chevronWidth: number,
  actionsWidth: number,
  gap: number,
) {
  return (
    rowWidth > 0 &&
    labelWidth > 0 &&
    metaWidth > 0 &&
    rowWidth >= labelWidth + metaWidth + chevronWidth + actionsWidth + gap
  );
}

/** Bodies run to the right edge beside a corner cluster (floating controls
 * sit over them); landscape notches remain protected. */
export function workspaceBodyEdges(cornerSensor: boolean) {
  return cornerSensor
    ? (["bottom", "left"] as const)
    : (["bottom", "left", "right"] as const);
}

/** The same page content remains inline until the optional companion slot fits. */
export function workspacePanelContent(home: boolean, hasAgent: boolean) {
  return home ? "home" : hasAgent ? "agent" : "loading";
}

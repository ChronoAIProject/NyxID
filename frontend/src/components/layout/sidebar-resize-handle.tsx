import { useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { cn } from "@/lib/utils";
import {
  SIDEBAR_WIDTH,
  clampSidebarWidth,
  useThemeStore,
  type SidebarId,
} from "@/stores/theme-store";

const KEYBOARD_STEP = 16;

/**
 * Drag handle on a sidebar's right edge. While dragging, the live width is
 * reported through `onPreview` and committed to the display settings once on
 * release, so localStorage is written once per drag. Double-click resets;
 * arrow keys resize when focused.
 */
export function SidebarResizeHandle({
  sidebar,
  label,
  onPreview,
}: {
  readonly sidebar: SidebarId;
  readonly label: string;
  /** Live width during a drag; `null` when the drag ends. */
  readonly onPreview: (width: number | null) => void;
}) {
  const width = useThemeStore((s) => s.sidebarWidths[sidebar]);
  const setSidebarWidth = useThemeStore((s) => s.setSidebarWidth);
  const drag = useRef<{ startX: number; startWidth: number; current: number } | null>(null);
  const [dragging, setDragging] = useState(false);

  function handlePointerDown(e: PointerEvent<HTMLDivElement>) {
    if (e.button !== 0) return;
    e.preventDefault();
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { startX: e.clientX, startWidth: width, current: width };
    setDragging(true);
  }

  function handlePointerMove(e: PointerEvent<HTMLDivElement>) {
    if (!drag.current) return;
    const next = clampSidebarWidth(drag.current.startWidth + e.clientX - drag.current.startX);
    drag.current.current = next;
    onPreview(next);
  }

  function endDrag() {
    if (!drag.current) return;
    setSidebarWidth(sidebar, drag.current.current);
    drag.current = null;
    setDragging(false);
    onPreview(null);
  }

  function handleKeyDown(e: KeyboardEvent<HTMLDivElement>) {
    const delta = { ArrowLeft: -KEYBOARD_STEP, ArrowRight: KEYBOARD_STEP }[e.key];
    if (delta !== undefined) {
      e.preventDefault();
      setSidebarWidth(sidebar, width + delta);
    } else if (e.key === "Home" || e.key === "End") {
      e.preventDefault();
      setSidebarWidth(sidebar, e.key === "Home" ? SIDEBAR_WIDTH.min : SIDEBAR_WIDTH.max);
    }
  }

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-valuemin={SIDEBAR_WIDTH.min}
      aria-valuemax={SIDEBAR_WIDTH.max}
      aria-valuenow={width}
      tabIndex={0}
      title="Drag to resize · double-click to reset"
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onDoubleClick={() => setSidebarWidth(sidebar, SIDEBAR_WIDTH.default)}
      onKeyDown={handleKeyDown}
      className={cn(
        "group absolute inset-y-0 -right-1 z-20 w-2 cursor-col-resize touch-none outline-none",
        dragging && "select-none",
      )}
    >
      <span
        className={cn(
          "absolute inset-y-0 left-1/2 w-0.5 -translate-x-1/2 transition-colors",
          dragging
            ? "bg-input-focus"
            : "bg-transparent group-hover:bg-hairline-strong group-focus-visible:bg-input-focus",
        )}
      />
    </div>
  );
}

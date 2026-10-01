import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { GripVertical, MoveDiagonal2 } from "lucide-react";
import { useEffect, useRef, type PointerEvent, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { PANEL_HEIGHTS, panelSpan } from "@/lib/usage-analytics";
import type { AnalyticsPanel } from "@/schemas/usage-analytics";

const heights = ["compact", "standard", "tall"] as const;

export function SortablePanel({
  panel,
  disabled,
  compact,
  onResize,
  children,
}: {
  panel: AnalyticsPanel;
  disabled: boolean;
  compact: boolean;
  onResize: (panel: AnalyticsPanel) => void;
  children: (handle: ReactNode) => ReactNode;
}) {
  const itemRef = useRef<HTMLDivElement | null>(null);
  const cancelResizeRef = useRef<(() => void) | null>(null);
  const {
    attributes,
    listeners,
    setNodeRef,
    setActivatorNodeRef,
    transform,
    transition,
    isDragging,
    isOver,
  } = useSortable({ id: panel.id, disabled });
  useEffect(() => () => cancelResizeRef.current?.(), []);

  function columns() {
    const grid = itemRef.current?.parentElement;
    return grid
      ? getComputedStyle(grid).gridTemplateColumns.split(/\s+/).filter(Boolean)
          .length || 1
      : 1;
  }

  function resizeStart(event: PointerEvent<HTMLButtonElement>) {
    if (disabled || event.button !== 0 || !itemRef.current) return;
    event.preventDefault();
    event.stopPropagation();

    const item = itemRef.current;
    const grid = item.parentElement;
    if (!grid) return;
    const columnCount = columns();
    const gap = Number.parseFloat(getComputedStyle(grid).columnGap) || 0;
    const columnWidth =
      (grid.clientWidth - gap * (columnCount - 1)) / columnCount;
    const startWidth = item.getBoundingClientRect().width;
    const startSpan = panelSpan(panel);
    const startHeight = panel.height ?? (compact ? "compact" : "standard");
    const startX = event.clientX;
    const startY = event.clientY;
    let nextSpan: number = startSpan;
    let nextHeight: AnalyticsPanel["height"] = startHeight;

    item.dataset.resizing = "true";
    function move(pointer: globalThis.PointerEvent) {
      const dx = pointer.clientX - startX;
      nextSpan = startSpan;
      if (Math.abs(dx) > 12 && columnWidth > 0) {
        nextSpan =
          dx > 0 && startSpan >= columnCount
            ? startSpan
            : Math.max(
                1,
                Math.min(
                  columnCount,
                  Math.round((startWidth + dx + gap) / (columnWidth + gap)),
                ),
              );
      }
      const targetHeight =
        PANEL_HEIGHTS[startHeight] + pointer.clientY - startY;
      nextHeight = heights.reduce((nearest, height) =>
        Math.abs(PANEL_HEIGHTS[height] - targetHeight) <
        Math.abs(PANEL_HEIGHTS[nearest] - targetHeight)
          ? height
          : nearest,
      );
      item.dataset.span = String(nextSpan);
      item.style.setProperty(
        "--analytics-resize-height",
        `${PANEL_HEIGHTS[nextHeight]}px`,
      );
    }
    function detach() {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", cancel);
      window.removeEventListener("blur", cancel);
      cancelResizeRef.current = null;
      delete item.dataset.resizing;
      item.dataset.span = String(startSpan);
      item.style.removeProperty("--analytics-resize-height");
    }
    function end() {
      detach();
      if (nextSpan !== startSpan || nextHeight !== startHeight)
        onResize({
          ...panel,
          span: nextSpan as 1 | 2 | 3,
          wide: nextSpan === 3,
          height: nextHeight,
        });
    }
    function cancel() {
      detach();
    }
    cancelResizeRef.current = cancel;
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", cancel);
    window.addEventListener("blur", cancel);
  }

  function resizeByKeyboard(key: string) {
    const span = panelSpan(panel);
    const height = panel.height ?? (compact ? "compact" : "standard");
    const heightIndex = heights.indexOf(height);
    const nextSpan =
      key === "ArrowLeft"
        ? Math.max(1, span - 1)
        : key === "ArrowRight"
          ? Math.min(columns(), span + 1)
          : span;
    const nextHeight =
      key === "ArrowUp"
        ? heights[Math.max(0, heightIndex - 1)]
        : key === "ArrowDown"
          ? heights[Math.min(heights.length - 1, heightIndex + 1)]
          : height;
    if (nextSpan !== span || nextHeight !== height)
      onResize({
        ...panel,
        span: nextSpan as 1 | 2 | 3,
        wide: nextSpan === 3,
        height: nextHeight,
      });
  }

  return (
    <div
      ref={(node) => {
        itemRef.current = node;
        setNodeRef(node);
      }}
      className="analytics-grid-item min-w-0"
      data-span={panelSpan(panel)}
      data-panel-id={panel.id}
      data-drop-target={isOver && !isDragging ? "true" : undefined}
      style={{
        transform: CSS.Translate.toString(transform),
        transition,
        opacity: isDragging ? 0.35 : 1,
      }}
    >
      {children(
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              ref={setActivatorNodeRef}
              variant="ghost"
              size="icon"
              className="analytics-drag-handle size-7 shrink-0 touch-none cursor-grab text-muted-foreground active:cursor-grabbing"
              disabled={disabled}
              {...attributes}
              {...listeners}
              aria-label={`Drag ${panel.title}`}
            >
              <GripVertical className="size-3.5" />
            </Button>
          </TooltipTrigger>
          <TooltipContent side="top">Drag to reorder</TooltipContent>
        </Tooltip>,
      )}
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            type="button"
            variant="ghost"
            size="icon"
            className="analytics-resize-handle absolute bottom-1 right-1 z-10 size-7 touch-none text-muted-foreground"
            disabled={disabled}
            aria-label={`Resize ${panel.title}`}
            onPointerDown={resizeStart}
            onKeyDown={(event) => {
              if (!event.key.startsWith("Arrow")) return;
              event.preventDefault();
              event.stopPropagation();
              resizeByKeyboard(event.key);
            }}
          >
            <MoveDiagonal2 style={{ width: 16, height: 16 }} />
          </Button>
        </TooltipTrigger>
        <TooltipContent side="top">Drag to resize</TooltipContent>
      </Tooltip>
    </div>
  );
}

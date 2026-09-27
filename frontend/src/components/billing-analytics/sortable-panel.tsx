import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { GripVertical } from "lucide-react";
import type { ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { panelSpan } from "@/lib/usage-analytics";
import type { AnalyticsPanel } from "@/schemas/usage-analytics";

export function SortablePanel({
  panel,
  disabled,
  children,
}: {
  panel: AnalyticsPanel;
  disabled: boolean;
  children: (handle: ReactNode) => ReactNode;
}) {
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
  return (
    <div
      ref={setNodeRef}
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
        <Button
          ref={setActivatorNodeRef}
          variant="ghost"
          size="icon"
          className="size-7 shrink-0 touch-none cursor-grab text-muted-foreground active:cursor-grabbing"
          disabled={disabled}
          {...attributes}
          {...listeners}
          aria-label={`Drag ${panel.title}`}
          title="Drag to reorder. Space to pick up, arrow keys to move, Space to drop."
        >
          <GripVertical className="size-3.5" />
        </Button>,
      )}
    </div>
  );
}

import { createContext, useContext, type ReactNode } from "react";
import { GripVertical } from "lucide-react";
import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { Button } from "@/components/ui/button";

type DragHandle = Pick<
  ReturnType<typeof useSortable>,
  "attributes" | "listeners" | "setActivatorNodeRef"
>;
const DragHandleContext = createContext<DragHandle | null>(null);
export function ServiceOrderRow({
  id,
  ordering,
  blocked,
  children,
}: {
  readonly id: string;
  readonly ordering: boolean;
  readonly blocked: boolean;
  readonly children: ReactNode;
}) {
  if (!ordering)
    return <tbody data-service-connection-group={id}>{children}</tbody>;
  return (
    <SortableServiceRow id={id} blocked={blocked}>
      {children}
    </SortableServiceRow>
  );
}
function SortableServiceRow({
  id,
  blocked,
  children,
}: {
  readonly id: string;
  readonly blocked: boolean;
  readonly children: ReactNode;
}) {
  const {
    attributes,
    listeners,
    setActivatorNodeRef,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id, disabled: blocked });
  return (
    <DragHandleContext.Provider
      value={{ attributes, listeners, setActivatorNodeRef }}
    >
      <tbody
        ref={setNodeRef}
        data-service-connection-group={id}
        data-sortable-connection={id}
        style={{
          transform: CSS.Transform.toString(transform),
          transition,
          opacity: isDragging ? 0.35 : undefined,
        }}
      >
        {children}
      </tbody>
    </DragHandleContext.Provider>
  );
}
export function ServiceOrderHandle({
  label,
  disabled,
}: {
  readonly label: string;
  readonly disabled: boolean;
}) {
  const handle = useContext(DragHandleContext);
  if (!handle) return null;
  const { setActivatorNodeRef, attributes, listeners } = handle;
  return (
    <Button
      ref={setActivatorNodeRef}
      type="button"
      variant="ghost"
      size="icon"
      className="shrink-0 touch-none cursor-grab active:cursor-grabbing"
      disabled={disabled}
      aria-label={`Drag ${label}`}
      {...attributes}
      {...listeners}
    >
      <GripVertical className="size-4" aria-hidden="true" />
    </Button>
  );
}

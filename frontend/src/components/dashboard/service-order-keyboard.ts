import type { KeyboardCoordinateGetter } from "@dnd-kit/core";

export const serviceOrderKeyboardCoordinates: KeyboardCoordinateGetter = (
  event,
  { context },
) => {
  if (event.code !== "ArrowUp" && event.code !== "ArrowDown") return;
  event.preventDefault();
  const { active, over, collisionRect, droppableRects, droppableContainers } =
    context;
  if (!active || !collisionRect) return;
  const items = active.data.current?.sortable?.items as string[] | undefined;
  if (!items) return;
  const index = items.indexOf(String(over?.id ?? active.id));
  const next = items[index + (event.code === "ArrowUp" ? -1 : 1)];
  if (!next || droppableContainers.get(next)?.disabled) return;
  const rect = droppableRects.get(next);
  if (!rect) return;
  // Center the complete row group on its neighbour, including an open panel.
  return {
    x: collisionRect.left,
    y: rect.top + (rect.height - collisionRect.height) / 2,
  };
};

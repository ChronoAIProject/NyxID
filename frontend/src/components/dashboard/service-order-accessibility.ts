import type { DndContextProps } from "@dnd-kit/core";
import { translateUi } from "@/i18n/use-ui-translation";

export function serviceOrderAccessibility(
  connections: readonly { id: string; label: string }[],
  ids: string[],
): NonNullable<DndContextProps["accessibility"]> {
  return {
    screenReaderInstructions: {
      draggable: translateUi(
        "Press Space to pick up a connection, arrow keys to move it, Space to drop, or Escape to cancel.",
      ),
    },
    announcements: {
      onDragStart: ({ active }) =>
        translateUi("Picked up {{label}}, position {{position}} of {{count}}", {
          label: connections.find((key) => key.id === active.id)?.label,
          position: ids.indexOf(String(active.id)) + 1,
          count: ids.length,
        }),
      onDragOver: ({ active, over }) =>
        over && active.id !== over.id
          ? translateUi("Moved to position {{position}}", {
              position: ids.indexOf(String(over.id)) + 1,
            })
          : undefined,
      onDragEnd: ({ over }) =>
        over
          ? translateUi("Dropped at position {{position}}", {
              position: ids.indexOf(String(over.id)) + 1,
            })
          : translateUi("Cancelled"),
      onDragCancel: () => translateUi("Cancelled"),
    },
  };
}

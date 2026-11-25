import type { DndContextProps } from "@dnd-kit/core";
import { translateUi } from "@/i18n/use-ui-translation";
import type { AnalyticsPanel } from "@/schemas/usage-analytics";

export function panelDragAccessibility(
  panels: AnalyticsPanel[],
): NonNullable<DndContextProps["accessibility"]> {
  return {
    screenReaderInstructions: {
      draggable: translateUi(
        "Press Space to pick up a panel, arrow keys to move it, Space to drop, or Escape to cancel.",
      ),
    },
    announcements: {
      onDragStart: ({ active }) =>
        translateUi("Picked up {{title}}.", {
          title:
            panels.find((p) => p.id === active.id)?.title ??
            translateUi("panel"),
        }),
      onDragOver: ({ over }) =>
        over
          ? translateUi("Over position {{position}} of {{count}}.", {
              position: panels.findIndex((p) => p.id === over.id) + 1,
              count: panels.length,
            })
          : translateUi("Outside the panel grid."),
      onDragEnd: ({ over }) =>
        over
          ? translateUi("Dropped at position {{position}}.", {
              position: panels.findIndex((p) => p.id === over.id) + 1,
            })
          : translateUi("Move cancelled."),
      onDragCancel: () => translateUi("Move cancelled."),
    },
  };
}

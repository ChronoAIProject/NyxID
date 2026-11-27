import { act, render, screen } from "@testing-library/react";
import { DndContext, type DragEndEvent } from "@dnd-kit/core";
import { SortableContext } from "@dnd-kit/sortable";
import { afterEach, expect, it, vi } from "vitest";
import { appI18n } from "@/i18n";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { newPanel } from "@/lib/usage-analytics";
import { TooltipProvider } from "@/components/ui/tooltip";
import { SortablePanel } from "./sortable-panel";
import { panelDragAccessibility } from "./panel-drag-accessibility";

afterEach(async () => {
  await appI18n.changeLanguage("en");
});
const panels = [
  newPanel({ title: "User's Traffic 面板" }),
  newPanel({ title: "Second" }),
  newPanel({ title: "Third" }),
];
const event = (over: string | null = panels[2]!.id) =>
  ({
    active: { id: panels[0]!.id },
    over: over ? { id: over } : null,
  }) as DragEndEvent;

it("resolves all announcements in the current language without translating panel data", async () => {
  const a = panelDragAccessibility(panels).announcements!;
  expect(a.onDragStart(event())).toBe("Picked up User's Traffic 面板.");
  expect(a.onDragOver!(event())).toBe("Over position 3 of 3.");
  expect(a.onDragEnd(event())).toBe("Dropped at position 3.");
  expect(a.onDragOver!(event(null))).toBe("Outside the panel grid.");
  expect(a.onDragEnd(event(null))).toBe("Move cancelled.");
  expect(a.onDragCancel(event())).toBe("Move cancelled.");
  await appI18n.changeLanguage("zh-CN");
  expect(a.onDragStart(event())).toBe("已选取 User's Traffic 面板。");
  expect(a.onDragOver!(event())).toBe("位于第 3 个位置，共 3 个。");
  expect(a.onDragEnd(event())).toBe("已放到第 3 个位置。");
  expect(a.onDragOver!(event(null))).toBe("已移出面板网格。");
  expect(a.onDragEnd(event(null))).toBe("已取消移动。");
  expect(a.onDragCancel(event())).toBe("已取消移动。");
  expect(panelDragAccessibility([]).announcements!.onDragStart(event())).toBe(
    `已选取 ${translateUi("panel")}。`,
  );
});

it("renders localized hidden instructions and the real sortable handle after a language switch", async () => {
  function Canvas() {
    useUiTranslation();
    return (
      <TooltipProvider>
        <DndContext accessibility={panelDragAccessibility(panels)}>
          <SortableContext items={panels.map((p) => p.id)}>
            <SortablePanel
              panel={panels[0]!}
              disabled={false}
              compact
              onResize={vi.fn()}
            >
              {(handle) => <div>{handle}</div>}
            </SortablePanel>
          </SortableContext>
        </DndContext>
      </TooltipProvider>
    );
  }
  const view = render(<Canvas />);
  const handle = screen.getByRole("button", {
    name: "Drag User's Traffic 面板",
  });
  expect(handle).toHaveAttribute("aria-roledescription", "sortable");
  expect(handle).toHaveAccessibleDescription(
    "Press Space to pick up a panel, arrow keys to move it, Space to drop, or Escape to cancel.",
  );
  await act(async () => {
    await appI18n.changeLanguage("zh-CN");
  });
  expect(handle).toHaveAttribute("aria-roledescription", "可排序");
  expect(handle).toHaveAccessibleDescription(
    "按空格键选取面板，使用方向键移动，再按空格键放下，或按 Escape 键取消。",
  );
  const hidden = document.getElementById(
    handle.getAttribute("aria-describedby")!,
  )!;
  expect(hidden).toHaveTextContent("按空格键选取面板");
  expect(hidden).not.toBeVisible();
  view.unmount();
});

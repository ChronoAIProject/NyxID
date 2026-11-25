import { act, render, screen } from "@testing-library/react";
import { DndContext, type DragEndEvent } from "@dnd-kit/core";
import { SortableContext } from "@dnd-kit/sortable";
import { afterEach, expect, it } from "vitest";
import { appI18n } from "@/i18n";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { ServiceOrderRow, ServiceOrderHandle } from "./service-order-rows";
import { serviceOrderAccessibility } from "./service-order-accessibility";

afterEach(async () => {
  await appI18n.changeLanguage("en");
});
const connections = [
  { id: "a", label: "User's API 123" },
  { id: "b", label: "Second" },
  { id: "c", label: "Third" },
];
const ids = connections.map((c) => c.id);
const event = (over: string | null = "c") =>
  ({ active: { id: "a" }, over: over ? { id: over } : null }) as DragEndEvent;

it("localizes every ordering announcement at call time and keeps connection labels verbatim", async () => {
  const a = serviceOrderAccessibility(connections, ids).announcements!;
  expect(a.onDragStart(event())).toBe(
    "Picked up User's API 123, position 1 of 3",
  );
  expect(a.onDragOver!(event())).toBe("Moved to position 3");
  expect(a.onDragEnd(event())).toBe("Dropped at position 3");
  expect(a.onDragOver!(event("a"))).toBeUndefined();
  expect(a.onDragOver!(event(null))).toBeUndefined();
  expect(a.onDragEnd(event(null))).toBe("Cancelled");
  expect(a.onDragCancel(event())).toBe("Cancelled");
  await appI18n.changeLanguage("zh-CN");
  expect(a.onDragStart(event())).toBe(
    "已选取 User's API 123，位于第 1 个位置，共 3 个",
  );
  expect(a.onDragOver!(event())).toBe("已移到第 3 个位置");
  expect(a.onDragEnd(event())).toBe("已放到第 3 个位置");
  expect(a.onDragEnd(event(null))).toBe("已取消");
  expect(a.onDragCancel(event())).toBe("已取消");
});

it("renders Chinese instructions and sortable roles on the real connection row", async () => {
  function Table() {
    useUiTranslation();
    return (
      <DndContext accessibility={serviceOrderAccessibility(connections, ids)}>
        <SortableContext items={ids}>
          <table>
            <ServiceOrderRow id="a" ordering blocked={false}>
              <tr>
                <td>
                  <ServiceOrderHandle
                    label={connections[0]!.label}
                    disabled={false}
                  />
                </td>
              </tr>
            </ServiceOrderRow>
          </table>
        </SortableContext>
      </DndContext>
    );
  }
  const view = render(<Table />);
  const handle = screen.getByRole("button", { name: "Drag User's API 123" });
  expect(handle).toHaveAttribute("aria-roledescription", "sortable");
  await act(async () => {
    await appI18n.changeLanguage("zh-CN");
  });
  expect(handle).toHaveAttribute("aria-roledescription", "可排序");
  expect(handle).toHaveAccessibleDescription(
    "按空格键选取连接，使用方向键移动，再按空格键放下，或按 Escape 键取消。",
  );
  const hidden = document.getElementById(
    handle.getAttribute("aria-describedby")!,
  )!;
  expect(hidden).toHaveTextContent("按空格键选取连接");
  expect(hidden).not.toBeVisible();
  view.unmount();
});

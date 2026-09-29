import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { newPanel } from "@/lib/usage-analytics";
import { TooltipProvider } from "@/components/ui/tooltip";
import { SortablePanel } from "./sortable-panel";

vi.mock("@dnd-kit/sortable", () => ({
  useSortable: () => ({
    attributes: {},
    listeners: {},
    setNodeRef: vi.fn(),
    setActivatorNodeRef: vi.fn(),
    transform: null,
    transition: undefined,
    isDragging: false,
    isOver: false,
  }),
}));

function renderPanel(onResize = vi.fn()) {
  const panel = newPanel({ title: "Traffic" });
  const view = render(
    <TooltipProvider>
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "100px 100px 100px",
          columnGap: 12,
        }}
      >
        <SortablePanel
          panel={panel}
          disabled={false}
          compact
          onResize={onResize}
        >
          {(handle) => <div>{handle}</div>}
        </SortablePanel>
      </div>
    </TooltipProvider>,
  );
  const item = view.container.querySelector<HTMLElement>(
    ".analytics-grid-item",
  )!;
  Object.defineProperty(item.parentElement, "clientWidth", {
    configurable: true,
    value: 324,
  });
  vi.spyOn(item, "getBoundingClientRect").mockReturnValue({
    width: 100,
  } as DOMRect);
  return { item, panel, onResize };
}

it("drags a panel to a wider column span and taller chart height", () => {
  const { item, onResize } = renderPanel();
  const handle = screen.getByRole("button", { name: "Resize Traffic" });
  fireEvent.pointerDown(handle, { button: 0, clientX: 100, clientY: 100 });
  fireEvent.pointerMove(window, { clientX: 225, clientY: 180 });
  expect(item).toHaveAttribute("data-span", "2");
  expect(item.style.getPropertyValue("--analytics-resize-height")).toBe(
    "280px",
  );
  fireEvent.pointerUp(window);
  expect(onResize).toHaveBeenCalledWith(
    expect.objectContaining({ span: 2, wide: false, height: "standard" }),
  );
});

it("supports keyboard sizing and cancels an interrupted drag", () => {
  const { item, onResize } = renderPanel();
  const handle = screen.getByRole("button", { name: "Resize Traffic" });
  fireEvent.keyDown(handle, { key: "ArrowRight" });
  expect(onResize).toHaveBeenCalledWith(
    expect.objectContaining({ span: 2, height: "compact" }),
  );
  onResize.mockClear();
  fireEvent.pointerDown(handle, { button: 0, clientX: 100, clientY: 100 });
  fireEvent.pointerMove(window, { clientX: 225, clientY: 180 });
  fireEvent.pointerCancel(window);
  expect(item).toHaveAttribute("data-span", "1");
  expect(onResize).not.toHaveBeenCalled();
});

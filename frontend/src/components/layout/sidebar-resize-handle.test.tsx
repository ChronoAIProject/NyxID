import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useThemeStore } from "@/stores/theme-store";
import { SidebarResizeHandle } from "./sidebar-resize-handle";

beforeEach(() => {
  localStorage.clear();
  useThemeStore.getState().resetDisplay();
  // happy-dom lacks pointer capture.
  HTMLElement.prototype.setPointerCapture = vi.fn();
});

describe("SidebarResizeHandle", () => {
  it("previews while dragging and commits once on release", () => {
    const onPreview = vi.fn();
    render(<SidebarResizeHandle sidebar="assistant" label="Resize sidebar" onPreview={onPreview} />);
    const handle = screen.getByRole("separator", { name: "Resize sidebar" });

    fireEvent.pointerDown(handle, { button: 0, clientX: 200, pointerId: 1 });
    fireEvent.pointerMove(handle, { clientX: 260, pointerId: 1 });
    expect(onPreview).toHaveBeenLastCalledWith(260);
    expect(useThemeStore.getState().sidebarWidths.assistant).toBe(200);

    fireEvent.pointerMove(handle, { clientX: 900, pointerId: 1 });
    expect(onPreview).toHaveBeenLastCalledWith(360);
    fireEvent.pointerUp(handle, { pointerId: 1 });
    expect(useThemeStore.getState().sidebarWidths.assistant).toBe(360);
    expect(onPreview).toHaveBeenLastCalledWith(null);
  });

  it("resizes with the keyboard and resets on double-click", () => {
    render(<SidebarResizeHandle sidebar="dashboard" label="Resize sidebar" onPreview={vi.fn()} />);
    const handle = screen.getByRole("separator", { name: "Resize sidebar" });
    fireEvent.keyDown(handle, { key: "ArrowRight" });
    expect(handle).toHaveAttribute("aria-valuenow", "216");
    fireEvent.keyDown(handle, { key: "Home" });
    expect(useThemeStore.getState().sidebarWidths.dashboard).toBe(180);
    fireEvent.doubleClick(handle);
    expect(useThemeStore.getState().sidebarWidths.dashboard).toBe(200);
  });
});

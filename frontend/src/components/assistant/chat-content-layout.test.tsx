import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useThemeStore } from "@/stores/theme-store";
import { ChatContentLayout } from "./chat-content-layout";

beforeEach(() => {
  useThemeStore.getState().resetDisplay();
  HTMLElement.prototype.setPointerCapture = vi.fn();
});
afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

it("coalesces observer measurements outside delivery and cancels them on unmount", () => {
  let notify: ResizeObserverCallback;
  let boundary: Element;
  const frames = new Map<number, FrameRequestCallback>();
  let nextFrame = 0;
  vi.stubGlobal("requestAnimationFrame", vi.fn((callback: FrameRequestCallback) => {
    frames.set(++nextFrame, callback);
    return nextFrame;
  }));
  vi.stubGlobal("cancelAnimationFrame", vi.fn((id: number) => frames.delete(id)));
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: ResizeObserverCallback) { notify = callback; }
    observe(element: Element) { boundary = element; }
    unobserve() {}
    disconnect() {}
  });
  const { unmount } = render(<ChatContentLayout>Chat</ChatContentLayout>);
  const readWidth = vi.fn(() => 900);
  Object.defineProperty(boundary!, "clientWidth", { get: readWidth });
  act(() => {
    notify!([], {} as ResizeObserver);
    notify!([], {} as ResizeObserver);
  });
  expect(readWidth).not.toHaveBeenCalled();
  expect(frames.size).toBe(1);
  act(() => {
    const queued = [...frames.values()];
    frames.clear();
    queued.forEach(frame => frame(0));
  });
  expect(readWidth).toHaveBeenCalledTimes(1);
  act(() => notify!([], {} as ResizeObserver));
  expect(frames.size).toBe(1);
  unmount();
  expect(frames.size).toBe(0);
});

it("previews a centered resize and saves it only when the pointer is released", () => {
  const { container } = render(<ChatContentLayout>Chat</ChatContentLayout>);
  const handle = screen.getByRole("separator", { name: "Resize chat content from right" });
  const stored = localStorage.getItem("nyxid.theme");
  fireEvent.pointerDown(handle, { pointerId: 1, button: 0, clientX: 800 });
  fireEvent.pointerMove(handle, { pointerId: 1, clientX: 900 });
  expect(container.firstElementChild).toHaveStyle({ "--chat-content-width": "958px" });
  expect(localStorage.getItem("nyxid.theme")).toBe(stored);
  fireEvent.pointerUp(handle, { pointerId: 1 });
  expect(useThemeStore.getState().chatContentWidth).toBe(958);
});

it.each(["pointerCancel", "lostPointerCapture", "escape"])("discards a resize on %s", (event) => {
  const { container } = render(<ChatContentLayout>Chat</ChatContentLayout>);
  const handle = screen.getByRole("separator", { name: "Resize chat content from left" });
  const stored = localStorage.getItem("nyxid.theme");
  fireEvent.pointerDown(handle, { pointerId: 1, button: 0, clientX: 300 });
  fireEvent.pointerMove(handle, { pointerId: 1, clientX: 200 });
  expect(container.firstElementChild).toHaveStyle({ "--chat-content-width": "958px" });
  if (event === "escape") fireEvent.keyDown(handle, { key: "Escape" });
  else if (event === "pointerCancel") fireEvent.pointerCancel(handle, { pointerId: 1 });
  else fireEvent.lostPointerCapture(handle, { pointerId: 1 });
  fireEvent.pointerUp(handle, { pointerId: 1 });
  expect(container.firstElementChild).toHaveStyle({ "--chat-content-width": "758px" });
  expect(localStorage.getItem("nyxid.theme")).toBe(stored);
});

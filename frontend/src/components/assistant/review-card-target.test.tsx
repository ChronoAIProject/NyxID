import { StrictMode } from "react";
import { act, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { ReviewCardNavigation, ReviewCardTarget } from "./review-card-target";

afterEach(() => { vi.useRealTimers(); vi.restoreAllMocks(); });

it("scrolls and briefly highlights the selected card after it mounts", () => {
  vi.useFakeTimers();
  const scroll = vi.spyOn(HTMLElement.prototype, "scrollIntoView").mockImplementation(() => {});
  const view = render(<ReviewCardTarget selected={false}><p>Draft</p></ReviewCardTarget>);
  act(() => vi.advanceTimersByTime(20));
  expect(scroll).not.toHaveBeenCalled();
  view.rerender(<ReviewCardTarget selected><p>Draft</p></ReviewCardTarget>);
  act(() => vi.advanceTimersByTime(20));
  expect(scroll).toHaveBeenCalledWith({ block: "center", behavior: "instant" });
  expect(screen.getByText("Draft").parentElement).toHaveClass("ring-2");
  view.rerender(<ReviewCardTarget selected><p>Updated draft</p></ReviewCardTarget>);
  act(() => vi.advanceTimersByTime(4000));
  expect(scroll).toHaveBeenCalledTimes(1);
  expect(screen.getByText("Updated draft").parentElement).not.toHaveClass("ring-2");
});


it("starts the highlight timeout when a delayed animation frame becomes visible", () => {
  vi.useFakeTimers();
  let frame: FrameRequestCallback | undefined;
  vi.spyOn(globalThis, "requestAnimationFrame").mockImplementation((callback) => {
    frame = callback;
    return 1;
  });
  vi.spyOn(HTMLElement.prototype, "scrollIntoView").mockImplementation(() => {});
  render(<ReviewCardTarget selected><p>Delayed draft</p></ReviewCardTarget>);
  act(() => vi.advanceTimersByTime(5000));
  expect(screen.getByText("Delayed draft").parentElement).not.toHaveClass("ring-2");
  act(() => frame?.(5000));
  expect(screen.getByText("Delayed draft").parentElement).toHaveClass("ring-2");
  act(() => vi.advanceTimersByTime(4000));
  expect(screen.getByText("Delayed draft").parentElement).not.toHaveClass("ring-2");
});

it("reveals a pending card once per navigation, including StrictMode and row remounts", async () => {
  const scroll = vi.spyOn(HTMLElement.prototype, "scrollIntoView");
  const tree = (show: boolean, card = "first") => <StrictMode>
    <ReviewCardNavigation selectionKey={card}>
      {show ? <ReviewCardTarget selected><p>Review this action</p></ReviewCardTarget> : null}
    </ReviewCardNavigation>
  </StrictMode>;
  const frame = async () => act(async () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve())));
  try {
    const view = render(tree(true));
    await frame();
    expect(scroll).toHaveBeenCalledTimes(1);
    expect(view.container.querySelector(".ring-input-focus")).not.toBeNull();
    view.rerender(tree(false));
    view.rerender(tree(true));
    await frame();
    expect(scroll).toHaveBeenCalledTimes(1);
    expect(view.container.querySelector(".ring-input-focus")).toBeNull();
    view.rerender(tree(true, "second"));
    await frame();
    expect(scroll).toHaveBeenCalledTimes(2);
    view.rerender(tree(true, "first"));
    await frame();
    expect(scroll).toHaveBeenCalledTimes(3);
  } finally { scroll.mockRestore(); }
});

import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { GuidedTour } from "./guided-tour";

const steps = [
  { target: "composer", title: "Ask NyxBot anything", body: "Type a request." },
  { target: "missing", title: "Not on screen", body: "Skipped." },
  { target: "approvals", title: "Approvals", body: "Decisions wait here." },
];

beforeEach(() => {
  // jsdom has no layout; give the targets a box.
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    x: 10, y: 10, top: 10, left: 10, right: 110, bottom: 50, width: 100, height: 40, toJSON: () => ({}),
  } as DOMRect);
  globalThis.ResizeObserver ??= class {
    observe() {}
    disconnect() {}
    unobserve() {}
  } as unknown as typeof ResizeObserver;
  Element.prototype.scrollIntoView ??= () => undefined;
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

it("walks the on-screen targets and skips ones that are not there", async () => {
  const user = userEvent.setup();
  const onClose = vi.fn();
  const screenTargets = (
    <>
      <div data-tour="composer">composer</div>
      <a data-tour="approvals" href="/assistant/approvals">Approvals</a>
    </>
  );
  // The tour opens over a screen that is already rendered.
  const { rerender } = render(screenTargets);
  rerender(<>{screenTargets}<GuidedTour steps={steps} onClose={onClose} /></>);
  expect(screen.getByRole("dialog", { name: "Ask NyxBot anything" })).toBeTruthy();
  expect(screen.getByText("1 of 2")).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Next" }));
  expect(screen.getByRole("dialog", { name: "Approvals" })).toBeTruthy();
  expect(screen.queryByText("Not on screen")).toBeNull();
  await user.click(screen.getByRole("button", { name: "Done" }));
  expect(onClose).toHaveBeenCalledTimes(1);
});

it("Escape ends the tour", async () => {
  const user = userEvent.setup();
  const onClose = vi.fn();
  const target = <div data-tour="composer">composer</div>;
  const { rerender } = render(target);
  rerender(<>{target}<GuidedTour steps={steps} onClose={onClose} /></>);
  await user.keyboard("{Escape}");
  expect(onClose).toHaveBeenCalledTimes(1);
});

import { act, cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { useSectionLayout } from "./use-section-layout";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
it("follows the settings container at 42rem, including a resize inside a wide viewport", () => {
  let width = 671;
  let resize = () => {};
  const disconnect = vi.fn();
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(callback: () => void) {
        resize = callback;
      }
      observe() {}
      disconnect = disconnect;
    },
  );
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
    () => ({ width }) as DOMRect,
  );
  function View() {
    const [setContainer, wide] = useSectionLayout();
    return (
      <div ref={setContainer} data-testid="surface" data-wide={String(wide)} />
    );
  }
  const view = render(<View />);
  expect(screen.getByTestId("surface")).toHaveAttribute("data-wide", "false");
  act(() => {
    width = 672;
    resize();
  });
  expect(screen.getByTestId("surface")).toHaveAttribute("data-wide", "true");
  act(() => {
    width = 400;
    resize();
  });
  expect(screen.getByTestId("surface")).toHaveAttribute("data-wide", "false");
  view.unmount();
  expect(disconnect).toHaveBeenCalledOnce();
});

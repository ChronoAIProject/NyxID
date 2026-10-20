import { act, render } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { useAssistantViewport } from "./use-assistant-viewport";

function Shell() {
  const ref = useAssistantViewport();
  return <div ref={ref} data-testid="shell" />;
}

afterEach(() => vi.unstubAllGlobals());

it("tracks keyboard resize and viewport pan, preserves zoom, and restores document scrolling", () => {
  const viewport = Object.assign(new EventTarget(), {
    height: 800,
    offsetTop: 0,
    scale: 1,
  });
  vi.stubGlobal("visualViewport", viewport);
  const overflow = document.body.style.overflow;
  const overscroll = document.documentElement.style.overscrollBehavior;
  const { getByTestId, unmount } = render(<Shell />);
  const shell = getByTestId("shell");
  expect(shell.style.height).toBe("800px");
  expect(document.body.style.overflow).toBe("hidden");

  act(() => {
    viewport.height = 420;
    viewport.dispatchEvent(new Event("resize"));
    viewport.offsetTop = 60;
    viewport.dispatchEvent(new Event("scroll"));
  });
  expect(shell.style.height).toBe("420px");
  expect(shell.style.top).toBe("60px");

  act(() => {
    viewport.scale = 2;
    viewport.height = 210;
    viewport.dispatchEvent(new Event("resize"));
  });
  expect(shell.style.height).toBe("420px");

  act(() => {
    viewport.scale = 1;
    viewport.height = 800;
    viewport.offsetTop = 0;
    viewport.dispatchEvent(new Event("resize"));
  });
  expect(shell.style.height).toBe("800px");
  expect(shell.style.top).toBe("0px");
  unmount();
  expect(document.body.style.overflow).toBe(overflow);
  expect(document.documentElement.style.overscrollBehavior).toBe(overscroll);
  viewport.height = 500;
  viewport.dispatchEvent(new Event("resize"));
  expect(shell.style.height).toBe("800px");
});

it("uses the CSS viewport fallback without VisualViewport", () => {
  vi.stubGlobal("visualViewport", undefined);
  const { getByTestId, unmount } = render(<Shell />);
  expect(getByTestId("shell").style.height).toBe("");
  unmount();
});

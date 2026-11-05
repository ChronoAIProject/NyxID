import { act, cleanup, renderHook } from "@testing-library/react";
import { useLayoutEffect } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { usePanelVisibility } from "./use-panel-visibility";

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

it.each([
  [[false, true], true, true],
  [[true, false], false, true],
  [[false, false], false, false],
] as const)("uses the latest batched viewport record in %j", (states, visible, hasEntered) => {
  let callback!: IntersectionObserverCallback;
  const observe = vi.fn();
  const disconnect = vi.fn();
  vi.stubGlobal("IntersectionObserver", class {
    constructor(handler: IntersectionObserverCallback) { callback = handler; }
    observe = observe;
    disconnect = disconnect;
  });
  const element = document.createElement("div");
  const entry = (isIntersecting: boolean): IntersectionObserverEntry => ({
    isIntersecting,
    target: element,
    boundingClientRect: element.getBoundingClientRect(),
    intersectionRect: element.getBoundingClientRect(),
    intersectionRatio: isIntersecting ? 1 : 0,
    rootBounds: null,
    time: performance.now(),
  });
  const { result, unmount } = renderHook(() => {
    const panel = usePanelVisibility();
    useLayoutEffect(() => { panel.ref.current = element; }, [panel.ref]);
    return panel;
  });
  expect(observe).toHaveBeenCalledWith(element);
  act(() => callback(states.map(entry), {} as IntersectionObserver));
  expect(result.current.visible).toBe(visible);
  expect(result.current.hasEntered).toBe(hasEntered);
  act(() => callback([entry(false)], {} as IntersectionObserver));
  expect(result.current.visible).toBe(false);
  expect(result.current.hasEntered).toBe(hasEntered);
  unmount();
  expect(disconnect).toHaveBeenCalledOnce();
});

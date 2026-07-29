import { act, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useFadingPresence } from "./use-fading-presence";

afterEach(() => {
  vi.useRealTimers();
});

describe("useFadingPresence", () => {
  it("enters on the next task and remains present through the exit window", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(
      ({ active }) => useFadingPresence(active, 500),
      { initialProps: { active: false } },
    );

    expect(result.current).toEqual({ present: false, visible: false });

    rerender({ active: true });
    expect(result.current).toEqual({ present: true, visible: false });
    act(() => vi.advanceTimersByTime(0));
    expect(result.current).toEqual({ present: true, visible: true });

    rerender({ active: false });
    expect(result.current).toEqual({ present: true, visible: false });
    act(() => vi.advanceTimersByTime(499));
    expect(result.current.present).toBe(true);
    act(() => vi.advanceTimersByTime(1));
    expect(result.current).toEqual({ present: false, visible: false });
  });

  it("cancels an in-flight exit when presence becomes active again", () => {
    vi.useFakeTimers();
    const { result, rerender } = renderHook(
      ({ active }) => useFadingPresence(active, 500),
      { initialProps: { active: true } },
    );

    act(() => vi.advanceTimersByTime(0));
    rerender({ active: false });
    act(() => vi.advanceTimersByTime(250));
    rerender({ active: true });
    act(() => vi.advanceTimersByTime(500));

    expect(result.current).toEqual({ present: true, visible: true });
  });
});

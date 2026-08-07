import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useSmoothReveal } from "./use-smooth-reveal";

/** Roughly one frame at 60Hz, which is what the fake clock drives rAF at. */
const FRAME_MS = 16;

function runFrames(count: number): void {
  for (let frame = 0; frame < count; frame += 1) {
    act(() => {
      vi.advanceTimersByTime(FRAME_MS);
    });
  }
}

function recordFrames(count: number, read: () => string): string[] {
  const outputs: string[] = [];
  for (let frame = 0; frame < count; frame += 1) {
    runFrames(1);
    outputs.push(read());
  }
  return outputs;
}

/**
 * Frames until the reveal catches up. The first frame only establishes the
 * clock baseline — there is no previous timestamp to measure against — so a
 * converged reveal always costs at least two.
 */
function framesToConverge(text: string): number {
  const { result, rerender, unmount } = renderHook(
    ({ value }: { value: string }) => useSmoothReveal(value, true),
    { initialProps: { value: "" } },
  );
  rerender({ value: text });
  let frames = 0;
  while (result.current !== text && frames < 500) {
    runFrames(1);
    frames += 1;
  }
  unmount();
  return frames;
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("useSmoothReveal", () => {
  it("returns the whole text when the block is not streaming", () => {
    const { result } = renderHook(() => useSmoothReveal("Settled answer.", false));
    expect(result.current).toBe("Settled answer.");
  });

  it("withholds arrived characters while streaming, then converges", () => {
    const text = "a".repeat(400);
    const { result, rerender } = renderHook(
      ({ value }: { value: string }) => useSmoothReveal(value, true),
      { initialProps: { value: "" } },
    );

    rerender({ value: text });
    runFrames(2);
    // Paced, not painted whole: a 400-character chunk does not land at once.
    expect(result.current.length).toBeGreaterThan(0);
    expect(result.current.length).toBeLessThan(text.length);

    // ...and it is never anything but a prefix of what actually arrived.
    expect(text.startsWith(result.current)).toBe(true);

    runFrames(30);
    expect(result.current).toBe(text);
  });

  it("makes the wait grow with the logarithm of the backlog, not with it", () => {
    const short = framesToConverge("b".repeat(200));
    const long = framesToConverge("b".repeat(1200));
    // The drain rate is proportional to the backlog, so the backlog decays
    // exponentially and the wait grows logarithmically: six times the text
    // costs well under twice the frames. A fixed characters-per-second rate —
    // the obvious implementation — would have cost six times as long, which is
    // exactly how a typewriter effect makes long answers crawl.
    expect(long).toBeLessThan(short * 2);
    // And neither is allowed to feel like a stall.
    expect(long * FRAME_MS).toBeLessThan(500);
  });

  it("snaps a backlog too large to be a stream", () => {
    const bulk = "c".repeat(4000);
    const { result, rerender } = renderHook(
      ({ value }: { value: string }) => useSmoothReveal(value, true),
      { initialProps: { value: "" } },
    );
    rerender({ value: bulk });
    // A re-mount or history projection is not typing; it appears at once, on
    // the first frame that carries a clock delta.
    runFrames(2);
    expect(result.current).toBe(bulk);
  });

  it("never slices past a block whose text was replaced with a shorter one", () => {
    const { result, rerender } = renderHook(
      ({ value }: { value: string }) => useSmoothReveal(value, true),
      { initialProps: { value: "d".repeat(500) } },
    );
    runFrames(30);
    rerender({ value: "short" });
    runFrames(1);
    expect(result.current).toBe("short");
  });

  it("hands over the full text the moment the block settles", () => {
    const text = "e".repeat(600);
    const { result, rerender } = renderHook(
      ({ value, active }: { value: string; active: boolean }) =>
        useSmoothReveal(value, active),
      { initialProps: { value: "", active: true } },
    );
    rerender({ value: text, active: true });
    runFrames(1);
    expect(result.current.length).toBeLessThan(text.length);

    rerender({ value: text, active: false });
    // No content may be left stranded behind the animation when a turn ends.
    expect(result.current).toBe(text);
  });

  it("does not pace when the reader asked for reduced motion", () => {
    const matchMedia = vi
      .spyOn(window, "matchMedia")
      .mockReturnValue({ matches: true } as MediaQueryList);
    const text = "f".repeat(800);
    const { result, rerender } = renderHook(
      ({ value }: { value: string }) => useSmoothReveal(value, true),
      { initialProps: { value: "" } },
    );
    rerender({ value: text });
    expect(result.current).toBe(text);
    matchMedia.mockRestore();
  });

  it("never paints an incomplete emphasis run", () => {
    const { result, rerender } = renderHook(
      ({ value }: { value: string }) => useSmoothReveal(value, true),
      { initialProps: { value: "" } },
    );
    const outputs = [result.current];

    rerender({ value: "make it **bo" });
    outputs.push(result.current, ...recordFrames(20, () => result.current));
    rerender({ value: "make it **bold** please" });
    outputs.push(result.current, ...recordFrames(30, () => result.current));

    for (const output of outputs) {
      const completeRuns = output.match(/\*\*/g)?.length ?? 0;
      expect(completeRuns === 0 || completeRuns >= 2, output).toBe(true);
    }
    expect(result.current).toBe("make it **bold** please");
  });

  it("never paints part of a grapheme cluster", () => {
    const text = "Hi 👩‍💻!";
    const { result, rerender } = renderHook(
      ({ value }: { value: string }) => useSmoothReveal(value, true),
      { initialProps: { value: "" } },
    );
    rerender({ value: text });

    const outputs = recordFrames(20, () => result.current);
    for (const output of outputs) {
      expect(output).not.toMatch(/[\uD800-\uDBFF]$/);
      if (output.includes("👩")) expect(output).toContain("👩‍💻");
    }
    expect(result.current).toBe(text);
  });

  it("never retracts when the inline scan window slides", () => {
    const initial = `${"a".repeat(45)}\`${"b".repeat(164)}\`${"c".repeat(90)}`;
    expect(initial).toHaveLength(301);
    const { result, rerender } = renderHook(
      ({ value }: { value: string }) => useSmoothReveal(value, true),
      { initialProps: { value: "" } },
    );
    rerender({ value: initial });
    runFrames(40);
    expect(result.current).toBe(initial);

    const outputs = [result.current];
    rerender({ value: `${initial}!` });
    outputs.push(result.current, ...recordFrames(3, () => result.current));
    for (let index = 1; index < outputs.length; index += 1) {
      expect(outputs[index]?.startsWith(outputs[index - 1] ?? "")).toBe(true);
    }
  });

  it("resets the display clamp for a same-length replacement", () => {
    const { result, rerender } = renderHook(
      ({ value }: { value: string }) => useSmoothReveal(value, true),
      { initialProps: { value: "plain text!" } },
    );
    expect(result.current).toBe("plain text!");

    rerender({ value: "**open tail" });
    expect(result.current).toBe("");
  });

  it("reveals a withheld unsafe tail synchronously on settle", () => {
    const { result, rerender } = renderHook(
      ({ value, active }: { value: string; active: boolean }) =>
        useSmoothReveal(value, active),
      { initialProps: { value: "", active: true } },
    );
    rerender({ value: "**open", active: true });
    runFrames(10);
    expect(result.current).toBe("");

    rerender({ value: "**open", active: false });
    expect(result.current).toBe("**open");
  });

  it("spreads later chunks over the observed arrival cadence", () => {
    const { result, rerender } = renderHook(
      ({ value }: { value: string }) => useSmoothReveal(value, true),
      { initialProps: { value: "" } },
    );
    rerender({ value: "a".repeat(60) });
    runFrames(20);
    rerender({ value: "a".repeat(120) });
    runFrames(20);
    rerender({ value: "a".repeat(180) });

    runFrames(2);
    const early = result.current.length;
    runFrames(6);
    expect(result.current.length).toBeGreaterThan(early);
    expect(result.current.length).toBeLessThan(180);
  });

  it("switches back to the legacy drain after producer idle", () => {
    const { result, rerender } = renderHook(
      ({ value }: { value: string }) => useSmoothReveal(value, true),
      { initialProps: { value: "" } },
    );
    rerender({ value: "a".repeat(60) });
    runFrames(20);
    rerender({ value: "a".repeat(120) });
    runFrames(20);
    rerender({ value: "a".repeat(1020) });

    runFrames(38);
    const beforeAdaptiveFrame = result.current.length;
    runFrames(1);
    const adaptiveAdvance = result.current.length - beforeAdaptiveFrame;
    runFrames(1);
    const beforeLegacyFrame = result.current.length;
    runFrames(1);
    const legacyAdvance = result.current.length - beforeLegacyFrame;

    expect(legacyAdvance).toBeGreaterThan(adaptiveAdvance);
    runFrames(25);
    expect(result.current).toBe("a".repeat(1020));
  });

  it("snaps a projection-sized jump after cadence tracking is armed", () => {
    const { result, rerender } = renderHook(
      ({ value }: { value: string }) => useSmoothReveal(value, true),
      { initialProps: { value: "" } },
    );
    rerender({ value: "a".repeat(60) });
    runFrames(19);
    rerender({ value: "a".repeat(120) });
    rerender({ value: "a".repeat(4120) });

    runFrames(1);
    expect(result.current).toBe("a".repeat(4120));
  });

  it("discards stale cadence after a producer stall", () => {
    const { result, rerender } = renderHook(
      ({ value }: { value: string }) => useSmoothReveal(value, true),
      { initialProps: { value: "" } },
    );
    rerender({ value: "a".repeat(60) });
    runFrames(20);
    rerender({ value: "a".repeat(120) });
    runFrames(100);
    rerender({ value: "a".repeat(180) });

    runFrames(8);
    expect(result.current).toBe("a".repeat(180));
  });

  it("keeps the adaptive floor time-derived at high refresh rates", () => {
    vi.stubGlobal(
      "requestAnimationFrame",
      (callback: FrameRequestCallback) =>
        window.setTimeout(() => callback(performance.now()), 8),
    );
    vi.stubGlobal("cancelAnimationFrame", (frame: number) => {
      window.clearTimeout(frame);
    });
    const advance = (milliseconds: number) => {
      act(() => {
        vi.advanceTimersByTime(milliseconds);
      });
    };
    try {
      const { result, rerender } = renderHook(
        ({ value }: { value: string }) => useSmoothReveal(value, true),
        { initialProps: { value: "" } },
      );
      rerender({ value: "a".repeat(60) });
      advance(320);
      rerender({ value: "a".repeat(120) });
      advance(600);

      expect(result.current.length).toBeLessThan(120);
      const before = result.current.length;
      advance(32);
      expect(result.current.length - before).toBeGreaterThan(0);
      expect(result.current.length - before).toBeLessThanOrEqual(2);
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

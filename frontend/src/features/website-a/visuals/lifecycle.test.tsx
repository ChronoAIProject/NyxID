import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { VisualMotionProvider } from "./motion-provider";
import { VisualLink, RevealFrame } from "./elements";
import { HeroSection } from "./hero";
import { loadBackgroundRuntime } from "./background-runtime";
import type { HeroScene } from "./background-runtime";
import { gsap } from "gsap";
import { StrictMode } from "react";

vi.mock("../website-context", () => ({
  useWebsite: () => ({ language: "en" }),
}));
vi.mock("./background-runtime", () => ({ loadBackgroundRuntime: vi.fn() }));

let reduced = false;
let hidden = false;
const mediaListeners = new Set<() => void>();
const observers: {
  target?: Element;
  callback: IntersectionObserverCallback;
  disconnect: ReturnType<typeof vi.fn<() => void>>;
}[] = [];
function viewport(visible: boolean) {
  act(() => {
    for (const observer of [...observers]) {
      if (observer.target)
        observer.callback(
          [
            {
              target: observer.target,
              isIntersecting: visible,
            } as IntersectionObserverEntry,
          ],
          {} as IntersectionObserver,
        );
    }
  });
}
function reduce(value: boolean) {
  act(() => {
    reduced = value;
    mediaListeners.forEach((changed) => changed());
  });
}
function background(value: boolean) {
  act(() => {
    hidden = value;
    document.dispatchEvent(new Event("visibilitychange"));
  });
}
beforeEach(() => {
  reduced = false;
  hidden = false;
  observers.length = 0;
  mediaListeners.clear();
  vi.stubGlobal("matchMedia", () => ({
    get matches() {
      return reduced;
    },
    addEventListener: (_: string, fn: () => void) => mediaListeners.add(fn),
    removeEventListener: (_: string, fn: () => void) =>
      mediaListeners.delete(fn),
  }));
  vi.stubGlobal(
    "IntersectionObserver",
    class {
      entry: (typeof observers)[number];
      constructor(callback: IntersectionObserverCallback) {
        this.entry = {
          callback,
          disconnect: vi.fn(() => {
            this.entry.target = undefined;
          }),
        };
        observers.push(this.entry);
      }
      observe(target: Element) {
        this.entry.target = target;
      }
      disconnect() {
        this.entry.disconnect();
      }
    },
  );
  vi.spyOn(document, "hidden", "get").mockImplementation(() => hidden);
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  vi.spyOn(console, "warn").mockImplementation(() => {});
});
afterEach(() => {
  cleanup();
  gsap.globalTimeline.clear();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("native CTA decoration lifecycle", () => {
  it("uses a fresh GPU canvas during StrictMode replay and releases resources on unmount", () => {
    const buffers = vi.fn(),
      arrays = vi.fn(),
      programs = vi.fn(),
      lose = vi.fn();
    const canvases: HTMLCanvasElement[] = [];
    const contexts = new WeakMap<HTMLCanvasElement, WebGL2RenderingContext>();
    vi.mocked(HTMLCanvasElement.prototype.getContext).mockImplementation(
      function (this: HTMLCanvasElement, kind: string) {
        if (kind !== "webgl2") return null;
        const cached = contexts.get(this);
        if (cached) return cached;
        canvases.push(this);
        let lost = false;
        const methods = {
          createShader: () => ({}),
          createProgram: () => ({}),
          createBuffer: () => ({}),
          createVertexArray: () => ({}),
          getShaderParameter: () => !lost,
          getProgramParameter: () => !lost,
          getUniformLocation: () => null,
          deleteBuffer: buffers,
          deleteVertexArray: arrays,
          deleteProgram: programs,
          getExtension: () => ({
            loseContext: () => {
              lost = true;
              lose();
            },
          }),
        };
        const context = new Proxy(methods, {
          get: (object, key) =>
            key in object
              ? object[key as keyof typeof methods]
              : typeof key === "string" && /^[A-Z_]+$/.test(key)
                ? 1
                : () => {},
        }) as unknown as WebGL2RenderingContext;
        contexts.set(this, context);
        return context;
      },
    );
    vi.stubGlobal(
      "ResizeObserver",
      class {
        observe() {}
        disconnect() {}
      },
    );
    vi.spyOn(window, "requestAnimationFrame").mockReturnValue(1);
    const cancel = vi.spyOn(window, "cancelAnimationFrame");
    const { container, unmount } = render(
      <StrictMode>
        <VisualMotionProvider>
          <VisualLink effect="button" href="/register">
            Get started
          </VisualLink>
        </VisualMotionProvider>
      </StrictMode>,
    );
    viewport(true);
    expect(canvases).toHaveLength(2);
    expect(canvases[0]).not.toBe(canvases[1]);
    expect(container.querySelectorAll("canvas")).toHaveLength(1);
    expect(container.querySelector("canvas")).toHaveAttribute(
      "data-render-state",
      "ready",
    );
    expect(lose).toHaveBeenCalledTimes(1);
    fireEvent(
      container.querySelector("canvas")!,
      new Event("webglcontextlost"),
    );
    expect(container.querySelector("canvas")).toHaveAttribute(
      "data-render-state",
      "lost",
    );
    expect(screen.getByRole("link", { name: "Get started" })).toBeVisible();
    unmount();
    expect(buffers).toHaveBeenCalledTimes(2);
    expect(arrays).toHaveBeenCalledTimes(2);
    expect(programs).toHaveBeenCalledTimes(2);
    expect(lose).toHaveBeenCalledTimes(2);
    expect(cancel).toHaveBeenCalled();
    expect(mediaListeners.size).toBe(0);
  });
  it("keeps the same anchor usable without WebGL and gates decoration on visibility, foreground and media changes", () => {
    const clicked = vi.fn();
    const { container, unmount } = render(
      <VisualMotionProvider>
        <VisualLink
          effect="button"
          href="/register?code=QA&return_to=%2Fkeys"
          onClick={clicked}
        >
          Get started
        </VisualLink>
      </VisualMotionProvider>,
    );
    const anchor = screen.getByRole("link", { name: "Get started" });
    expect(container.querySelector("canvas")).toBeNull();
    viewport(true);
    expect(container.querySelector("canvas")).toHaveAttribute(
      "data-render-state",
      "fallback",
    );
    expect(anchor).toHaveAttribute(
      "href",
      "/register?code=QA&return_to=%2Fkeys",
    );
    fireEvent.click(anchor);
    expect(clicked).toHaveBeenCalledOnce();
    expect(container.querySelectorAll("a,button,[tabindex]")).toHaveLength(1);
    viewport(false);
    expect(container.querySelector("canvas")).toBeNull();
    viewport(true);
    background(true);
    expect(container.querySelector("canvas")).toBeNull();
    background(false);
    expect(container.querySelector("canvas")).not.toBeNull();
    reduce(true);
    expect(container.querySelector("canvas")).toBeNull();
    expect(anchor).toBeVisible();
    reduce(false);
    expect(container.querySelector("canvas")).not.toBeNull();
    unmount();
    expect(mediaListeners.size).toBe(0);
    expect(observers.every((o) => o.disconnect.mock.calls.length === 1)).toBe(
      true,
    );
  });

  it.each(["compile", "link"])(
    "releases partial shader resources and retains navigation on %s failure",
    (stage) => {
      const gl = {
        VERTEX_SHADER: 1,
        FRAGMENT_SHADER: 2,
        COMPILE_STATUS: 3,
        createShader: vi.fn(() => ({})),
        shaderSource: vi.fn(),
        compileShader: vi.fn(),
        getShaderParameter: vi.fn(() => stage !== "compile"),
        deleteShader: vi.fn(),
        createProgram: vi.fn(() => ({})),
        attachShader: vi.fn(),
        bindAttribLocation: vi.fn(),
        linkProgram: vi.fn(),
        getProgramParameter: vi.fn(() => false),
        deleteProgram: vi.fn(),
        getExtension: vi.fn(() => ({ loseContext: lose })),
      };
      const lose = vi.fn();
      vi.mocked(HTMLCanvasElement.prototype.getContext).mockReturnValue(
        gl as unknown as WebGL2RenderingContext,
      );
      const { container, unmount } = render(
        <VisualMotionProvider>
          <VisualLink effect="button" href="/login?code=QA">
            Log in
          </VisualLink>
        </VisualMotionProvider>,
      );
      viewport(true);
      expect(gl.deleteShader).toHaveBeenCalledTimes(2);
      expect(gl.deleteProgram).toHaveBeenCalledOnce();
      expect(lose).toHaveBeenCalledOnce();
      expect(container.querySelector("canvas")).toHaveAttribute(
        "data-render-state",
        "fallback",
      );
      expect(screen.getByRole("link", { name: "Log in" })).toHaveAttribute(
        "href",
        "/login?code=QA",
      );
      unmount();
      expect(lose).toHaveBeenCalledOnce();
    },
  );

  it("drives Spotlight from the actual anchor and releases pointer/focus animation on reduced motion", () => {
    let now = 0,
      nextId = 0;
    const frames = new Map<number, FrameRequestCallback>();
    vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
      const id = ++nextId;
      frames.set(id, callback);
      return id;
    });
    vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
    const { container, unmount } = render(
      <VisualMotionProvider>
        <VisualLink effect="card" href="/solutions">
          Existing card
        </VisualLink>
      </VisualMotionProvider>,
    );
    const anchor = screen.getByRole("link", { name: "Existing card" });
    vi.spyOn(anchor, "getBoundingClientRect").mockReturnValue({
      x: 0,
      y: 0,
      left: 0,
      top: 0,
      right: 200,
      bottom: 100,
      width: 200,
      height: 100,
      toJSON: () => ({}),
    });
    vi.spyOn(anchor, "matches").mockImplementation(
      (selector) => selector === ":focus-visible",
    );
    viewport(true);
    fireEvent.pointerMove(window, { clientX: 100, clientY: 50 });
    fireEvent.focusIn(anchor);
    act(() => {
      for (let i = 0; i < 25; i++) {
        now += 16;
        const pending = [...frames.values()];
        frames.clear();
        pending.forEach((callback) => callback(performance.now() + now));
      }
    });
    const light = container.querySelector<HTMLElement>(
      ".spotlight-card__light",
    )!;
    expect(Number(light.style.opacity)).toBeGreaterThan(0);
    expect(light.style.background).toContain("radial-gradient");
    expect(container.querySelectorAll("a,button,[tabindex]")).toHaveLength(1);
    reduce(true);
    expect(container.querySelector(".spotlight-card")).toBeNull();
    expect(frames.size).toBe(0);
    expect(anchor).toHaveAttribute("href", "/solutions");
    unmount();
    expect(mediaListeners.size).toBe(0);
  });
});

describe("presentation-only entrance", () => {
  it("starts at .72, pauses offscreen/background, completes once and never hides its content", () => {
    const { container } = render(
      <VisualMotionProvider>
        <RevealFrame style={{ marginTop: 16 }}>Presentation</RevealFrame>
      </VisualMotionProvider>,
    );
    const frame = container.querySelector<HTMLElement>("[data-visual-reveal]")!;
    expect(Number(frame.style.opacity)).toBeCloseTo(0.72);
    viewport(true);
    const timeline = gsap.getTweensOf(frame)[0]?.parent;
    expect(timeline).toBeTruthy();
    background(true);
    expect(timeline!.paused()).toBe(true);
    background(false);
    expect(timeline!.paused()).toBe(false);
    viewport(false);
    expect(timeline!.paused()).toBe(true);
    viewport(true);
    act(() => {
      timeline!.progress(1);
    });
    expect(frame).toHaveStyle({ opacity: "1", marginTop: "16px" });
    viewport(false);
    viewport(true);
    expect(frame).toHaveStyle({ opacity: "1" });
    expect(frame.style.filter).toBe("");
  });

  it("restores original inline style immediately on reduced motion and clears tween resources on unmount", () => {
    const { container, unmount } = render(
      <VisualMotionProvider>
        <RevealFrame style={{ marginTop: 16 }}>Readable</RevealFrame>
      </VisualMotionProvider>,
    );
    const frame = container.querySelector<HTMLElement>("[data-visual-reveal]")!;
    viewport(true);
    reduce(true);
    expect(frame.style.opacity).toBe("");
    expect(frame.style.visibility).toBe("");
    expect(frame.style.marginTop).toBe("16px");
    expect(gsap.getTweensOf(frame)).toHaveLength(0);
    reduce(false);
    expect(frame.style.opacity).toBe("");
    unmount();
    expect(gsap.getTweensOf(frame)).toHaveLength(0);
  });
});

describe("legacy hero lifecycle", () => {
  it("pauses when offscreen/background and destroys the scene on reduced motion", async () => {
    vi.useFakeTimers();
    const scene: HeroScene = { paused: false, destroy: vi.fn() };
    const recovered: HeroScene = { paused: false, destroy: vi.fn() };
    vi.mocked(loadBackgroundRuntime).mockResolvedValue({
      addScene: vi
        .fn()
        .mockResolvedValueOnce(scene)
        .mockResolvedValueOnce(recovered),
    });
    const { container } = render(
      <VisualMotionProvider>
        <HeroSection>Hero content</HeroSection>
      </VisualMotionProvider>,
    );
    viewport(true);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(250);
    });
    expect(container.querySelector(".nyx-hero-scene")).toHaveAttribute(
      "data-scene-state",
      "ready",
    );
    viewport(false);
    expect(scene.paused).toBe(true);
    viewport(true);
    expect(scene.paused).toBe(false);
    background(true);
    expect(scene.paused).toBe(true);
    reduce(true);
    expect(scene.destroy).toHaveBeenCalledOnce();
    expect(screen.getByText("Hero content")).toBeVisible();
    background(false);
    reduce(false);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(250);
    });
    expect(container.querySelector(".nyx-hero-scene")).toHaveAttribute(
      "data-scene-state",
      "ready",
    );
    expect(recovered.paused).toBe(false);
  });

  it("destroys a late asynchronous scene after route unmount and cancels timers", async () => {
    vi.useFakeTimers();
    let resolve!: (scene: HeroScene) => void;
    const result = new Promise<HeroScene>((done) => {
      resolve = done;
    });
    vi.mocked(loadBackgroundRuntime).mockResolvedValue({
      addScene: vi.fn(() => result),
    });
    const { unmount } = render(
      <VisualMotionProvider>
        <HeroSection>Hero content</HeroSection>
      </VisualMotionProvider>,
    );
    viewport(true);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(250);
    });
    unmount();
    const scene: HeroScene = { paused: false, destroy: vi.fn() };
    await act(async () => resolve(scene));
    expect(scene.destroy).toHaveBeenCalledOnce();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("leaves the static readable hero on network failure", async () => {
    vi.useFakeTimers();
    vi.mocked(loadBackgroundRuntime).mockRejectedValue(new Error("offline"));
    const { container } = render(
      <VisualMotionProvider>
        <HeroSection>Readable hero</HeroSection>
      </VisualMotionProvider>,
    );
    viewport(true);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(250);
    });
    expect(container.querySelector(".nyx-hero-scene")).toHaveAttribute(
      "data-scene-state",
      "fallback",
    );
    expect(screen.getByText("Readable hero")).toBeVisible();
  });
});

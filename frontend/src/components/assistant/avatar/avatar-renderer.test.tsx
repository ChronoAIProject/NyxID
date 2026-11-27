import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { drawAvatar } from "../../../../../shared/agent-avatars/draw";
import { defaultAvatar } from "@/lib/assistant/agent-avatar-model";
import {
  AvatarRenderer as Renderer,
  type AvatarRendererProps,
} from "./avatar-renderer";
const AvatarRenderer = (props: AvatarRendererProps) => (
  <Renderer {...props} performance="expressive" />
);
import { avatarSchedulerInfo } from "./scheduler";
let visible = true,
  reduced = false,
  clock = 0,
  id = 0;
const frames = new Map<number, FrameRequestCallback>();
vi.mock("@/lib/assistant/use-avatar-environment", () => ({
  useAnimationActive: () => visible,
  useReducedAvatarMotion: () => reduced,
}));
const config = defaultAvatar("n", "nyxbot");
beforeEach(() => {
  visible = true;
  reduced = false;
  clock = 0;
  vi.spyOn(performance, "now").mockImplementation(() => clock);
  vi.stubGlobal("requestAnimationFrame", (fn: FrameRequestCallback) => {
    frames.set(++id, fn);
    return id;
  });
  vi.stubGlobal("cancelAnimationFrame", (key: number) => frames.delete(key));
});
afterEach(() => {
  cleanup();
  frames.clear();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
function advance(ms: number) {
  act(() => {
    for (let t = 0; t < ms; t += 100) {
      clock += 100;
      const batch = [...frames.values()];
      frames.clear();
      batch.forEach((f) => f(clock));
    }
  });
}
it("finishes one-shots, releases the scheduler, and does not replay on refocus", () => {
  const view = render(
    <AvatarRenderer
      config={config}
      pixels={44}
      mood="success"
      theme="dark"
      eventKey="reply-1"
    />,
  );
  expect(avatarSchedulerInfo().active).toBe(1);
  advance(1800);
  expect(avatarSchedulerInfo().active).toBe(0);
  expect(frames.size).toBe(0);
  visible = false;
  view.rerender(
    <AvatarRenderer
      config={config}
      pixels={44}
      mood="success"
      theme="dark"
      eventKey="reply-1"
    />,
  );
  visible = true;
  view.rerender(
    <AvatarRenderer
      config={config}
      pixels={44}
      mood="success"
      theme="dark"
      eventKey="reply-1"
    />,
  );
  expect(avatarSchedulerInfo().active).toBe(0);
  view.rerender(
    <AvatarRenderer
      config={config}
      pixels={44}
      mood="success"
      theme="dark"
      eventKey="reply-2"
    />,
  );
  expect(avatarSchedulerInfo().active).toBe(1);
  advance(1800);
  expect(avatarSchedulerInfo().active).toBe(0);
});
it("retains progress across interruption and applies live reduced motion", () => {
  const props = {
    config,
    pixels: 44,
    mood: "success" as const,
    theme: "dark" as const,
    eventKey: "reply",
  };
  const view = render(<AvatarRenderer {...props} />);
  advance(800);
  visible = false;
  view.rerender(<AvatarRenderer {...props} />);
  expect(avatarSchedulerInfo().active).toBe(0);
  visible = true;
  view.rerender(<AvatarRenderer {...props} />);
  advance(900);
  expect(avatarSchedulerInfo().active).toBe(0);
  view.rerender(<AvatarRenderer {...props} mood="thinking" />);
  reduced = true;
  view.rerender(<AvatarRenderer {...props} mood="thinking" />);
  expect(avatarSchedulerInfo().active).toBe(0);
});
it("level updates preserve the running clock and offscreen avatars do no work", () => {
  const props = {
    config,
    pixels: 44,
    mood: "speaking" as const,
    theme: "dark" as const,
  };
  const view = render(<AvatarRenderer {...props} level={0.1} />);
  advance(600);
  const registrations = avatarSchedulerInfo().registrations;
  const before = view.container.querySelector("g g")?.getAttribute("transform");
  view.rerender(<AvatarRenderer {...props} level={0.9} />);
  expect(
    view.container.querySelector("g g")?.getAttribute("transform"),
  ).not.toBe(before);
  expect(avatarSchedulerInfo().registrations).toBe(registrations);
  expect(avatarSchedulerInfo().active).toBe(1);
  visible = false;
  view.rerender(<AvatarRenderer {...props} level={0.9} />);
  expect(avatarSchedulerInfo().active).toBe(0);
});

it("starts a new state even when its caller retains the previous event key", () => {
  const props = {
    config,
    pixels: 44,
    theme: "dark" as const,
    eventKey: "turn",
  };
  const view = render(<AvatarRenderer {...props} mood="success" />);
  advance(1800);
  expect(avatarSchedulerInfo().active).toBe(0);
  view.rerender(<AvatarRenderer {...props} mood="thinking" />);
  expect(avatarSchedulerInfo().active).toBe(1);
});

it("clamps a negative first RAF timestamp and isolates failed callbacks", async () => {
  const { animateAvatar } = await import("./scheduler");
  clock = 1000;
  let seconds = -1,
    healthy = 0;
  const stop = animateAvatar(false, (t) => {
    seconds = t;
    healthy++;
  });
  const stopFailed = animateAvatar(false, () => {
    throw Error("renderer failed");
  });
  const beforeFailures = avatarSchedulerInfo().failures;
  const batch = [...frames.values()];
  frames.clear();
  batch.forEach((f) => f(900));
  expect(seconds).toBe(0);
  expect(avatarSchedulerInfo().failures).toBe(beforeFailures + 1);
  expect(avatarSchedulerInfo().active).toBe(1);
  expect(frames.size).toBe(1);
  advance(200);
  expect(healthy).toBeGreaterThan(1);
  stop();
  stopFailed();
  expect(frames.size).toBe(0);
});

it("preserves a claimed creation one-shot through visibility pauses without replay", async () => {
  const { recordCreatedAvatar, clearCreatedAvatars } =
    await import("../../../../../shared/agent-avatars/created");
  recordCreatedAvatar("new");
  const props = { config, agentId: "new", pixels: 44, theme: "dark" as const };
  const view = render(<AvatarRenderer {...props} />);
  advance(700);
  visible = false;
  view.rerender(<AvatarRenderer {...props} />);
  expect(
    view.container.querySelector('[data-avatar-state="created"]'),
  ).not.toBeNull();
  expect(avatarSchedulerInfo().active).toBe(0);
  visible = true;
  view.rerender(<AvatarRenderer {...props} />);
  advance(1200);
  expect(avatarSchedulerInfo().active).toBe(0);
  visible = false;
  view.rerender(<AvatarRenderer {...props} />);
  visible = true;
  view.rerender(<AvatarRenderer {...props} />);
  expect(avatarSchedulerInfo().active).toBe(0);
  clearCreatedAvatars();
});

it("retires creation when real work happens while the avatar is hidden", async () => {
  const { recordCreatedAvatar, clearCreatedAvatars } =
    await import("../../../../../shared/agent-avatars/created");
  recordCreatedAvatar("hidden-new");
  const props = {
    config,
    agentId: "hidden-new",
    pixels: 44,
    theme: "dark" as const,
  };
  const view = render(<AvatarRenderer {...props} />);
  advance(300);
  visible = false;
  view.rerender(<AvatarRenderer {...props} mood="working" />);
  view.rerender(<AvatarRenderer {...props} mood="idle" />);
  visible = true;
  view.rerender(<AvatarRenderer {...props} />);
  expect(
    view.container.querySelector('[data-avatar-state="created"]'),
  ).toBeNull();
  clearCreatedAvatars();
});

it("keeps compact character and dither surfaces while animating cached quiet frames", () => {
  for (const [surface, pixels] of [
    ["ascii-braille", 44],
    ["dither-ordered", 44],
    ["ascii-braille", 112],
  ] as const) {
    const textured = {
      ...config,
      choices: {
        ...config.choices,
        surface,
        motion: pixels >= 96 ? "quiet" : "automatic",
      },
    };
    const still = drawAvatar(textured, {
      size: pixels,
      mood: "thinking",
      theme: "dark",
    });
    const moving = drawAvatar(textured, {
      size: pixels,
      mood: "thinking",
      theme: "dark",
      seconds: 0.5,
    });
    expect(moving.surface).toBe(surface);
    expect(moving.motion.scale).toBeGreaterThan(1);
    expect(moving.mask).toBe(still.mask);
    expect(moving.backgroundPlane).toBe(still.backgroundPlane);
    expect(moving.foregroundPlane).toBe(still.foregroundPlane);
    expect(moving.paths.slice(0, -1)).toEqual(still.paths.slice(0, -1));
    for (let i = 0; i < still.paths.length - 1; i++)
      expect(moving.paths[i]).toBe(still.paths[i]);
    const view = render(
      <AvatarRenderer
        config={textured}
        pixels={pixels}
        mood="thinking"
        theme="dark"
      />,
    );
    expect(view.container.querySelector("svg")).toHaveAttribute(
      "data-face-surface",
      surface,
    );
    advance(600);
    expect(view.container.querySelector("svg")).toHaveAttribute(
      "data-face-surface",
      surface,
    );
    expect(view.container.querySelector("canvas")).toBeNull();
    view.unmount();
  }
});

it("animates idle textured avatars at home and chat sizes, and honours Still and reduced motion", () => {
  for (const pixels of [56, 72]) {
    const textured = {
      ...config,
      choices: { ...config.choices, surface: "ascii-braille" },
    };
    const still = drawAvatar(textured, { size: pixels, theme: "dark" });
    const moving = drawAvatar(textured, {
      size: pixels,
      theme: "dark",
      seconds: 0.5,
    });
    expect(moving.paths).toBe(still.paths);
    expect(moving.foregroundPlane).toBe(still.foregroundPlane);
    expect(moving.mask).toBe(still.mask);
    expect(moving.motion.scale).toBeGreaterThan(1.01);
    const props = { config: textured, pixels, theme: "dark" as const };
    const view = render(<AvatarRenderer {...props} />);
    const transform = () =>
      view.container.querySelector("g g")?.getAttribute("transform");
    const initial = transform();
    advance(600);
    expect(transform()).not.toBe(initial);
    expect(view.container.querySelector("svg")).toHaveAttribute(
      "data-face-surface",
      "ascii-braille",
    );
    expect(view.container.querySelector("canvas")).toBeNull();
    reduced = true;
    view.rerender(<AvatarRenderer {...props} />);
    expect(avatarSchedulerInfo().active).toBe(0);
    const reducedFrame = transform();
    advance(600);
    expect(transform()).toBe(reducedFrame);
    reduced = false;
    view.rerender(
      <AvatarRenderer
        {...props}
        config={{
          ...textured,
          choices: { ...textured.choices, motion: "still" },
        }}
      />,
    );
    expect(avatarSchedulerInfo().active).toBe(0);
    view.unmount();
  }
});

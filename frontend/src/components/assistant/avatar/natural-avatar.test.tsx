import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { defaultAvatar } from "@/lib/assistant/agent-avatar-model";
import { AvatarRenderer } from "./avatar-renderer";
import {
  RESTING_EYES,
  type EmotePose,
} from "../../../../../shared/agent-avatars/emotes";
import {
  propMaterial,
  staticPropPose,
} from "../../../../../shared/agent-avatars/props";
import {
  FACE_GEOMETRY,
  contrastRatio,
} from "../../../../../shared/agent-avatars/draw";

let reduced = false,
  visible = true;
const bridge = vi.hoisted(() => ({
  start: vi.fn(),
  state: vi.fn(),
  active: vi.fn(),
  stop: vi.fn(),
  listener: undefined as
    | { pose: (value: EmotePose) => void; ready: (ready: boolean) => void }
    | undefined,
}));
vi.mock("./rive-motion", () => ({ animateRiveEyes: bridge.start }));
vi.mock("@/lib/assistant/use-avatar-environment", () => ({
  useAnimationActive: () => visible,
  useReducedAvatarMotion: () => reduced,
}));
const config = defaultAvatar("nyx", "nyxbot");
beforeEach(() => {
  reduced = false;
  visible = true;
  vi.clearAllMocks();
  bridge.start.mockImplementation((listener) => {
    bridge.listener = listener;
    return { state: bridge.state, active: bridge.active, stop: bridge.stop };
  });
});
afterEach(cleanup);

it("uses Rive to change only the eye aperture, retaining the body and texture", () => {
  const textured = {
    ...config,
    choices: { ...config.choices, surface: "ascii-braille" },
  };
  const view = render(
    <AvatarRenderer
      config={textured}
      theme="dark"
      mood="thinking"
      pixels={112}
    />,
  );
  const eye = view.container.querySelector("[data-avatar-eye-mask]")!;
  const initial = eye.getAttribute("d");
  const body = view.container
    .querySelector("svg g g")!
    .getAttribute("transform");
  const paint = view.container.querySelector("image")?.getAttribute("href");
  act(() => {
    bridge.listener!.ready(true);
    bridge.listener!.pose({ x: -0.55, y: -0.65, openness: 0.9 });
  });
  expect(
    view.container.querySelector("[data-avatar-renderer]"),
  ).toHaveAttribute("data-avatar-renderer", "rive");
  expect(eye.getAttribute("d")).not.toBe(initial);
  expect(view.container.querySelector("svg")).toHaveAttribute(
    "data-face-surface",
    "ascii-braille",
  );
  expect(
    view.container.querySelector("svg g g")!.getAttribute("transform"),
  ).toBe(body);
  expect(view.container.querySelector("image")?.getAttribute("href")).toBe(
    paint,
  );
  act(() => bridge.listener!.pose(RESTING_EYES));
  expect(eye.getAttribute("d")).toBe(initial);
});

it("interrupts through the existing rig, pauses hidden work without replay, and honours reduced motion", () => {
  const props = { config, theme: "dark" as const, pixels: 56 };
  const view = render(<AvatarRenderer {...props} mood="thinking" />);
  act(() => bridge.listener!.ready(true));
  view.rerender(<AvatarRenderer {...props} mood="approval" />);
  expect(bridge.state).toHaveBeenLastCalledWith(5);
  expect(bridge.start).toHaveBeenCalledTimes(1);
  visible = false;
  view.rerender(<AvatarRenderer {...props} mood="approval" />);
  expect(bridge.active).toHaveBeenLastCalledWith(false);
  expect(bridge.stop).not.toHaveBeenCalled();
  visible = true;
  view.rerender(<AvatarRenderer {...props} mood="approval" />);
  expect(bridge.active).toHaveBeenLastCalledWith(true);
  expect(bridge.start).toHaveBeenCalledTimes(1);
  reduced = true;
  view.rerender(<AvatarRenderer {...props} mood="approval" />);
  expect(bridge.stop).toHaveBeenCalledTimes(1);
  expect(view.container.querySelector("[data-avatar-motion]")).toHaveAttribute(
    "data-avatar-motion",
    "static",
  );
});

it("does not load Rive for Still or small avatars, and keeps a complete face on load failure", () => {
  const props = {
    config: { ...config, choices: { ...config.choices, motion: "still" } },
    theme: "dark" as const,
    pixels: 112,
  };
  const view = render(<AvatarRenderer {...props} />);
  expect(bridge.start).not.toHaveBeenCalled();
  view.rerender(<AvatarRenderer {...props} config={config} pixels={20} />);
  expect(bridge.start).not.toHaveBeenCalled();
  view.rerender(<AvatarRenderer {...props} config={config} />);
  const eye = view.container.querySelector("[data-avatar-eye-mask]")!;
  const resting = eye.getAttribute("d");
  act(() => {
    bridge.listener!.ready(true);
    bridge.listener!.pose({ x: -0.5, y: -0.6, openness: 0.8 });
  });
  expect(eye.getAttribute("d")).not.toBe(resting);
  act(() => bridge.listener!.ready(false));
  expect(eye.getAttribute("d")).toBe(resting);
  expect(view.container.querySelector("svg")).not.toBeNull();
  expect(
    view.container.querySelector("[data-avatar-renderer]"),
  ).toHaveAttribute("data-avatar-renderer", "vector");
});

it.each(["ascii-outline", "ascii-inverted"])(
  "keeps %s eye edges still while Rive performs the activity props",
  (surface) => {
    const textured = { ...config, choices: { ...config.choices, surface } };
    const view = render(
      <AvatarRenderer config={textured} theme="dark" mood="thinking" />,
    );
    const eye = view.container.querySelector("[data-avatar-eye-mask]")!;
    const original = eye.getAttribute("d");
    act(() =>
      bridge.listener!.pose({
        x: 0.6,
        y: 0.3,
        openness: 0.5,
        props: staticPropPose("thinking"),
      }),
    );
    expect(bridge.start).toHaveBeenCalledOnce();
    expect(eye.getAttribute("d")).toBe(original);
    expect(
      view.container.querySelector("[data-avatar-motion-fallback]"),
    ).toHaveAttribute("data-avatar-motion-fallback", "texture");
    expect(view.container.querySelector("svg")).toHaveAttribute(
      "data-face-surface",
      surface,
    );
  },
);

it("applies Rive's coordinated keyboard/tap transforms without changing the saved face", () => {
  const view = render(
    <AvatarRenderer
      config={config}
      theme="dark"
      mood="drafting"
      pixels={112}
    />,
  );
  const keyboard = view.container.querySelector('[data-rive-prop="Keyboard"]')!;
  const tap = view.container.querySelector('[data-rive-prop="TapLeft"]')!;
  const still = staticPropPose("drafting");
  const before = tap.getAttribute("transform");
  act(() => {
    bridge.listener!.ready(true);
    bridge.listener!.pose({
      ...RESTING_EYES,
      props: { ...still, TapLeft: { ...still.TapLeft!, y: -13 } },
    });
  });
  expect(tap.getAttribute("transform")).not.toBe(before);
  expect(keyboard.getAttribute("transform")).toContain("scale(1 1)");
  expect(
    view.container.querySelector("svg[data-agent-avatar]"),
  ).toHaveAttribute("data-face-surface", "flat");
  reduced = true;
  view.rerender(
    <AvatarRenderer config={config} theme="dark" mood="waiting" pixels={112} />,
  );
  expect(bridge.stop).toHaveBeenCalledOnce();
  expect(
    view.container.querySelector('[data-rive-prop="Clock"]'),
  ).toHaveAttribute("transform", expect.stringContaining("scale(1 1)"));
  expect(
    view.container.querySelector('[data-rive-prop="Keyboard"]'),
  ).toHaveAttribute("transform", expect.stringContaining("scale(0 0)"));
});

it("uses compact props in chat, omits props at icon sizes and shows a representative still on failure", () => {
  const props = { config, theme: "dark" as const, mood: "working" as const };
  const view = render(<AvatarRenderer {...props} pixels={56} />);
  expect(view.container.querySelector("[data-avatar-props]")).toHaveAttribute(
    "data-prop-detail",
    "compact",
  );
  act(() => bridge.listener!.ready(false));
  expect(
    view.container.querySelector('[data-rive-prop="Tool"]'),
  ).toHaveAttribute("transform", expect.stringContaining("scale(1 1)"));
  view.rerender(<AvatarRenderer {...props} pixels={24} />);
  expect(view.container.querySelector("[data-avatar-props]")).toBeNull();
});

it("keeps props still in Quiet mode while retaining eye motion", () => {
  const quiet = { ...config, choices: { ...config.choices, motion: "quiet" } };
  const view = render(
    <AvatarRenderer config={quiet} theme="dark" mood="waiting" />,
  );
  const hand = view.container.querySelector('[data-rive-prop="Minute"]')!;
  const before = hand.getAttribute("transform");
  const pose = staticPropPose("waiting");
  act(() =>
    bridge.listener!.pose({
      ...RESTING_EYES,
      x: 0.4,
      props: { ...pose, Minute: { ...pose.Minute!, rotation: 2 } },
    }),
  );
  expect(hand.getAttribute("transform")).toBe(before);
});

it("uses the selected palette and texture for props, with readable marks across every palette", () => {
  for (const palette of Object.keys(FACE_GEOMETRY.palettes)) {
    const material = propMaterial(
      { ...config, choices: { ...config.choices, palette } },
      "dark",
    );
    expect(
      contrastRatio(material.ink, material.base),
      palette,
    ).toBeGreaterThanOrEqual(3);
  }
  const textured = {
    ...config,
    choices: {
      ...config.choices,
      palette: "core-mint",
      surface: "dither-ordered",
    },
  };
  const view = render(
    <AvatarRenderer config={textured} theme="dark" mood="drafting" />,
  );
  const props = view.container.querySelector("[data-avatar-props]")!;
  expect(props).toHaveAttribute("data-prop-palette", "core-mint");
  expect(props).toHaveAttribute("data-prop-surface", "dither-ordered");
  expect(props.querySelector("pattern")).not.toBeNull();
  const fill = props
    .querySelector('[data-rive-prop="Keyboard"] rect')
    ?.getAttribute("fill");
  view.rerender(
    <AvatarRenderer
      config={{
        ...textured,
        choices: { ...textured.choices, palette: "core-ocean" },
      }}
      theme="light"
      mood="drafting"
    />,
  );
  expect(props).toHaveAttribute("data-prop-palette", "core-ocean");
  expect(
    props
      .querySelector('[data-rive-prop="Keyboard"] rect')
      ?.getAttribute("fill"),
  ).not.toBe(fill);
});

it("releases a failed session, retries with a bound, and recovers without a reload", () => {
  vi.useFakeTimers();
  try {
    const view = render(
      <AvatarRenderer config={config} theme="dark" pixels={56} />,
    );
    act(() => bridge.listener!.ready(false));
    expect(bridge.stop).toHaveBeenCalledTimes(1);
    expect(
      view.container.querySelector("[data-avatar-renderer]"),
    ).toHaveAttribute("data-avatar-renderer", "vector");
    act(() => vi.advanceTimersByTime(2000));
    expect(bridge.start).toHaveBeenCalledTimes(2);
    act(() => bridge.listener!.ready(true));
    expect(
      view.container.querySelector("[data-avatar-renderer]"),
    ).toHaveAttribute("data-avatar-renderer", "rive");
    view.unmount();
    act(() => vi.advanceTimersByTime(10000));
    expect(bridge.start).toHaveBeenCalledTimes(2);
  } finally {
    vi.useRealTimers();
  }
});

it("cancels a pending recovery when unmounted", () => {
  vi.useFakeTimers();
  try {
    const view = render(
      <AvatarRenderer config={config} theme="dark" pixels={56} />,
    );
    act(() => bridge.listener!.ready(false));
    expect(bridge.stop).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(1);
    view.unmount();
    expect(vi.getTimerCount()).toBe(0);
    act(() => vi.advanceTimersByTime(10000));
    expect(bridge.start).toHaveBeenCalledTimes(1);
  } finally {
    vi.useRealTimers();
  }
});

it("stops retrying after two recovery attempts and retains the complete still", () => {
  vi.useFakeTimers();
  try {
    const view = render(
      <AvatarRenderer config={config} theme="dark" pixels={56} />,
    );
    for (const delay of [2000, 4000, 10000]) {
      act(() => bridge.listener!.ready(false));
      act(() => vi.advanceTimersByTime(delay));
    }
    expect(bridge.start).toHaveBeenCalledTimes(3);
    expect(bridge.stop).toHaveBeenCalledTimes(3);
    expect(
      view.container.querySelector("[data-avatar-renderer]"),
    ).toHaveAttribute("data-avatar-renderer", "vector");
    view.unmount();
  } finally {
    vi.useRealTimers();
  }
});

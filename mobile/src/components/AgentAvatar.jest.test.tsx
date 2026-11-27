import {
  recordCreatedAvatar,
  clearCreatedAvatars,
} from "../../../shared/agent-avatars/created";
import { AvatarAnimated } from "./avatar/AvatarArt";
import { defaultAvatar } from "../../../shared/agent-avatars/model";
import { AppState, StyleSheet } from "react-native";
import { configure, render, screen } from "@testing-library/react-native";
import { processColor } from "react-native";
import { withTiming } from "react-native-reanimated";
import type { ReactElement } from "react";
import { AgentAvatar, AgentAvatarStack } from "./AgentAvatar";
import { AgentAvatarsFlag } from "../lib/avatars/agentAvatarsFlag";
import { darkColors, lightColors } from "../theme/mobileTheme";
import { avatarBadge } from "../../../shared/agent-avatars/presentation";
import { quietActivityMotion, sampleActivityMotion } from "../../../shared/agent-avatars/quiet-motion";

// Avatars are decorative (hidden from assistive tech) unless labelled.
configure({ defaultIncludeHiddenElements: true });

let mockReducedMotion = false;
let mockAnimationActive = true;
let mockMode: "light" | "dark" = "light";
let mockPhase = 1;
jest.mock("../theme/ThemeContext", () => {
  const { darkColors, lightColors } = jest.requireActual("../theme/mobileTheme");
  return { useTheme: () => ({
    colors: mockMode === "dark" ? darkColors : lightColors,
    mode: mockMode,
  }) };
});
jest.mock("../lib/accessibility/useReducedMotion", () => ({
  useReducedMotion: () => mockReducedMotion,
}));
jest.mock("./avatar/useAvatarAnimationActive", () => ({
  useAvatarAnimationActive: () => mockAnimationActive,
}));
jest.mock("react-native-reanimated", () => ({
  ...require("react-native-reanimated/mock"),
  useSharedValue: (value: number) => require("react").useRef({ value }).current,
  withTiming: jest.fn(() => mockPhase),
}));

const AGENT = {
  id: "6f1c2a8e-3b4d-4c5e-8f90-123456789abc",
  name: "researcher",
  kind: "specialist" as const,
  display_name: "Research Desk",
};

const withFlag = (ui: ReactElement, on: boolean) =>
  render(<AgentAvatarsFlag value={on}>{ui}</AgentAvatarsFlag>);

beforeEach(() => {
  mockReducedMotion = false;
  mockAnimationActive = true;
  mockMode = "light";
  mockPhase = 1;
  jest.mocked(withTiming).mockClear();
});

test("flag off keeps today's avatar: tinted initials, NyxBot's mark", async () => {
  await withFlag(
    <AgentAvatar agent={AGENT} size="xl" mood="thinking" animated />,
    false,
  );
  expect(screen.getByText("RD")).toBeTruthy();
  expect(screen.queryByTestId("agent-avatar-still")).toBeNull();
  expect(screen.queryByTestId("agent-avatar-animated")).toBeNull();
  expect(screen.queryByTestId("avatar-activity-thinking")).toBeNull();
});

test.each(["light", "dark"] as const)("badges use shared geometry and %s theme tones", async (mode) => {
  mockMode = mode;
  const colors = mode === "dark" ? darkColors : lightColors;
  await withFlag(
    <>
      <AgentAvatar agent={AGENT} size="sm" mood="thinking" />
      <AgentAvatar agent={AGENT} size="md" mood="approval" />
      <AgentAvatar agent={AGENT} size="lg" mood="success" />
      <AgentAvatar agent={AGENT} size="xl" mood="stopping" />
    </>,
    true,
  );
  for (const [state, size, tint] of [
    ["thinking", 12, colors.primaryTone.text],
    ["approval", 12, colors.warningTone.text],
    ["success", 14, colors.successTone.text],
    ["stopping", 16, colors.textPrimary],
  ] as const) {
    const badge = screen.getByTestId(`avatar-activity-${state}`);
    expect(badge.props).toMatchObject({
      accessible: false, importantForAccessibility: "no", pointerEvents: "none",
    });
    const svg = badge.children[0];
    if (!svg || typeof svg === "string") throw new Error("Missing badge SVG");
    expect(svg.props).toMatchObject({ bbWidth: size, bbHeight: size, vbWidth: 24, vbHeight: 24 });
    const glyph = avatarBadge(state)!.glyph;
    const group = svg.children[0];
    if (!group || typeof group === "string") throw new Error("Missing badge paths");
    const disc = group.children[0];
    expect(disc && typeof disc !== "string" && disc.props.fill).toMatchObject({
      payload: processColor(colors.bg),
    });
    const path = group.children.find((child) => typeof child !== "string" && child.props.d === glyph.d);
    expect(path && typeof path !== "string" && path.props).toMatchObject({
      d: glyph.d,
      [glyph.strokeWidth ? "stroke" : "fill"]: { payload: processColor(tint) },
    });
  }
});

test.each(["thinking", "working", "drafting", "waiting", "approval", "retry", "stopping"] as const)(
  "%s samples the shared native face and ring at the shared period",
  async (mood) => {
    mockPhase = 0.25;
    const props = {
      config: defaultAvatar(AGENT.id, "specialist"),
      kind: "specialist",
      theme: "light" as const,
      pixels: 56,
      mood,
    };
    const result = await render(<AvatarAnimated {...props} />);
    // The mock advances the UI-thread phase in the effect. Render the sampled frame.
    await result.rerender(<AvatarAnimated {...props} />);
    const signature = quietActivityMotion(mood)!;
    if (signature.period === null) {
      expect(withTiming).not.toHaveBeenCalled();
    } else {
      expect(withTiming).toHaveBeenCalledWith(
        1, expect.objectContaining({ duration: signature.period * 1000 }),
      );
    }
    const sample = sampleActivityMotion(signature, (signature.period ?? 0) * mockPhase);
    const { x, y, rotate, scale } = sample.transform;
    expect(StyleSheet.flatten(screen.getByTestId("avatar-motion-face").props.style).transform)
      .toEqual([{ translateX: x * 56 / 512 }, { translateY: y * 56 / 512 }, { rotate: `${rotate}deg` }, { scale }]);
    expect(StyleSheet.flatten(screen.getByTestId(`avatar-accent-${mood}`).props.style).opacity)
      .toEqual(sample.ringOpacity);
  },
);

test("saved Still and inactive screens suppress quiet activity animation", async () => {
  const config = defaultAvatar(AGENT.id, "specialist");
  const art = { theme: "light" as const, pixels: 44, mood: "drafting" as const };
  const result = await render(
    <AvatarAnimated {...art} config={{ ...config, choices: { ...config.choices, react_drafting: "still" } }} />,
  );
  expect(withTiming).not.toHaveBeenCalled();
  expect(screen.queryByTestId("avatar-accent-drafting")).toBeNull();
  mockAnimationActive = false;
  await result.rerender(<AvatarAnimated {...art} config={config} />);
  expect(withTiming).not.toHaveBeenCalled();
  expect(StyleSheet.flatten(screen.getByTestId("avatar-motion-face").props.style).transform)
    .toEqual([{ translateX: 0 }, { translateY: -0 }, { rotate: "0deg" }, { scale: 1 }]);
});

test("flag on draws the face still at every size", async () => {
  await withFlag(
    <>
      <AgentAvatar agent={AGENT} size="xs" />
      <AgentAvatar agent={AGENT} size="lg" />
    </>,
    true,
  );
  expect(screen.queryByText("RD")).toBeNull();
  expect(screen.getAllByTestId("agent-avatar-still")).toHaveLength(2);
});

test("animated avatars move only at 24pt or more", async () => {
  await withFlag(
    <>
      <AgentAvatar agent={AGENT} size="sm" mood="thinking" animated />
      <AgentAvatar agent={AGENT} size="xl" mood="thinking" animated />
    </>,
    true,
  );
  expect(screen.getAllByTestId("agent-avatar-still")).toHaveLength(1);
  expect(screen.getAllByTestId("agent-avatar-animated")).toHaveLength(1);
});

test("Reduce Motion shows the mood's static pose with no motion", async () => {
  mockReducedMotion = true;
  await withFlag(
    <AgentAvatar agent={AGENT} size="xl" mood="working" animated />,
    true,
  );
  expect(screen.getByTestId("agent-avatar-static")).toBeTruthy();
  expect(screen.getByTestId("avatar-activity-working")).toBeTruthy();
  expect(withTiming).not.toHaveBeenCalled();
  expect(screen.queryByTestId("agent-avatar-animated")).toBeNull();
});

test("decorative unless labelled; a label makes it one image", async () => {
  await withFlag(
    <AgentAvatar agent={AGENT} size="xl" label="Avatar preview" />,
    true,
  );
  const image = screen.getByLabelText("Avatar preview");
  expect(image.props.accessibilityRole).toBe("image");
  expect(image.props.accessible).toBe(true);
  await screen.unmount();
  await withFlag(<AgentAvatar agent={AGENT} size="xl" />, true);
  // Unlabelled: hidden from VoiceOver and TalkBack.
  expect(
    screen.queryAllByRole("image", { includeHiddenElements: false }),
  ).toHaveLength(0);
  expect(screen.getByTestId("agent-avatar-still")).toBeTruthy();
});

test("stacks keep cutout discs behind faces", async () => {
  await withFlag(
    <AgentAvatarStack
      agents={[
        AGENT,
        { ...AGENT, id: "b" },
        { ...AGENT, id: "c" },
        { ...AGENT, id: "d" },
      ]}
      size="lg"
    />,
    true,
  );
  expect(screen.getAllByTestId("agent-avatar-still")).toHaveLength(3);
  expect(screen.getByText("+1")).toBeTruthy();
});

test("stills use the bounded 512-unit shared frame", async () => {
  await withFlag(<AgentAvatar agent={AGENT} size="lg" />, true);
  expect(screen.getByTestId("agent-avatar-still").props.style).toMatchObject({
    width: 32,
    height: 32,
  });
  const drawn = JSON.stringify(screen.toJSON());
  expect(drawn).toContain('"vbWidth":512');
  expect(drawn).toContain('"vbHeight":512');
});

test("a reused native renderer cannot borrow another claimed creation event", async () => {
  Object.defineProperty(AppState, "currentState", {
    value: "active",
    configurable: true,
  });
  recordCreatedAvatar("A");
  recordCreatedAvatar("B");
  const props = {
    config: defaultAvatar("A", "specialist"),
    kind: "specialist",
    pixels: 112,
    theme: "dark" as const,
  };
  const owner = await render(<AvatarAnimated {...props} agentId="B" />);
  const reused = await render(<AvatarAnimated {...props} agentId="A" />);
  expect(reused.queryAllByTestId("avatar-accent-assembly-pulse")).toHaveLength(
    1,
  );
  await reused.rerender(<AvatarAnimated {...props} agentId="B" />);
  expect(reused.queryAllByTestId("avatar-accent-assembly-pulse")).toHaveLength(
    0,
  );
  await reused.unmount();
  await owner.unmount();
  clearCreatedAvatars();
});

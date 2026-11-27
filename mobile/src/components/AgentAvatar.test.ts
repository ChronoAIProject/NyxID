import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import { hookHarness } from "../features/assistant/assistantHookTestHarness";
import * as model from "./agentAvatarModel";

type Element = { type: unknown; props: Record<string, any> };
function setup() {
  const harness = hookHarness<Element>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native": {
      View: "View",
      Text: "Text",
      Image: "Image",
      StyleSheet: { create: (value: unknown) => value },
    },
    "../theme/ThemeContext": {
      useTheme: () => ({
        mode: "dark",
        colors: {
          card: "#222225",
          border: "#333333",
          textSecondary: "#BABAC1",
        },
      }),
    },
    "../theme/designTokens": { fonts: { bodySemi: "semibold" } },
    "./agentAvatarModel": model,
    "../../assets/brand/nyxbot-avatar.png": "bundledNyxbotAvatar",
  });
  return harness.load(
    fileURLToPath(new URL("./AgentAvatar.tsx", import.meta.url)),
  ) as {
    AgentAvatar: (props: any) => Element;
    AgentAvatarStack: (props: any) => Element;
  };
}
test("agent identity uses display initials, exact mark, sizes and decorative semantics", () => {
  const { AgentAvatar } = setup();
  const agent = {
    id: "agent",
    name: "release-notes",
    kind: "specialist",
    display_name: " Knowledge editor ",
  };
  const decorative = AgentAvatar({ agent, size: "lg" });
  assert.equal(decorative.props.accessible, false);
  assert.equal(
    decorative.props.importantForAccessibility,
    "no-hide-descendants",
  );
  assert.equal(decorative.props.children.props.children, "KE");
  assert.equal(decorative.props.style[1].width, 32);
  const named = AgentAvatar({
    agent: { ...agent, display_name: " " },
    label: "Release notes",
  });
  assert.equal(named.props.accessibilityLabel, "Release notes");
  assert.equal(named.props.accessibilityRole, "image");
  assert.equal(named.props.children.props.children, "RN");
  const nyxbot = AgentAvatar({ agent: { ...agent, kind: "nyxbot" } });
  assert.equal(nyxbot.props.children.type, "Image");
  assert.equal(nyxbot.props.children.props.source, "bundledNyxbotAvatar");
  assert.deepEqual(
    { ...nyxbot.props.children.props.style },
    { width: "70%", height: "70%" },
  );
  assert.equal(nyxbot.props.children.props.accessible, false);
});
test("crew stacks cap real members and show the remaining count", () => {
  const { AgentAvatarStack } = setup();
  const stack = AgentAvatarStack({
    agents: ["a", "b", "c", "d", "e"].map((id) => ({
      id,
      name: id,
      kind: "specialist",
    })),
    max: 3,
  });
  assert.equal(stack.props.accessibilityElementsHidden, true);
  assert.equal(stack.props.children[0].length, 3);
  assert.equal(
    stack.props.children[1].props.children.props.children.props.children.join(
      "",
    ),
    "+2",
  );
});
